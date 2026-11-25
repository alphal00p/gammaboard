"""Real-instance native/process GammaLoop A/B benchmark, including observables.

Run with python -m benchmarks.gammaloop_ab --help. Every calibration and measured
case owns a fresh PostgreSQL/service/worker deployment. No synthetic evaluator is
used. The process child shares its parent's CPU allowance.
"""

import argparse
import csv
from datetime import datetime
import html
import hashlib
import json
import math
from pathlib import Path
import shutil
import socket
import statistics
import subprocess
import time

from . import common


def inline(value):
    """Encode the small JSON-compatible configuration subset as inline TOML."""
    if isinstance(value, dict):
        return "{ " + ", ".join(
            f"{json.dumps(k)} = {inline(v)}" for k, v in value.items() if v is not None
        ) + " }"
    if isinstance(value, list):
        return "[" + ", ".join(map(inline, value)) + "]"
    return json.dumps(value, allow_nan=False)


def card(backend, params, domain, worker, samples, batch_size, observable_format=None):
    evaluator = dict(kind="gammaloop", **params)
    if backend == "process":
        evaluator = dict(
            kind="process_evaluator", command=[str(worker)] + (["--observable-format", observable_format] if observable_format else []), cwd=str(worker.parent), args=params,
        )
    return f'''name = "gammaloop-{backend}"
evaluator = {inline(evaluator)}
evaluator_requirements = {{ bench_evaluator = 1 }}
sampler_requirements = {{ bench_sampler = 1 }}

[evaluator_runner_params]
performance_snapshot_interval_ms = 500
[sampler_aggregator_runner_params]
frontend_sync_interval_ms = 500
performance_snapshot_interval_ms = 500
[sampler_aggregator_runner_params.queue]
fixed_batch_size = {batch_size}
max_batch_size = {batch_size}
max_generation_size = 4096

[[task_queue]]
name = "train"
kind = "sample"
stop_condition = {{ max_samples = {samples['train']} }}
accumulator = {{ config = "gammaloop" }}
[task_queue.sampler.config]
kind = "havana_training"
seed = 41
bins = 32
samples_for_update = 2048
initial_training_rate = 0.5
final_training_rate = 0.1

[[task_queue]]
name = "inference"
kind = "sample"
stop_condition = {{ max_samples = {samples['inference']} }}
accumulator = {{ config = "gammaloop" }}
sampler = {{ from_stage = "train", config = {{ kind = "havana_inference", seed = 42 }} }}
'''


def state_hashes(directory):
    return {
        str(p.relative_to(directory)): common.file_hash(p)
        for p in sorted(directory.rglob("*")) if p.is_file()
    }


def seconds(start, end):
    return (datetime.fromisoformat(end) - datetime.fromisoformat(start)).total_seconds()


def scientific_result(results, phase, count):
    task = next(t for t in results["tasks"] if t["name"] == phase)
    output = task["snapshot"]["persisted_output"]
    histograms = output["bundle"]["histograms"]
    if not histograms or any(h["sample_count"] != count for h in histograms.values()):
        raise RuntimeError(f"{phase}: missing observables or incomplete histogram sample counts")
    integral = {}
    for component in output["estimate"]["components"]:
        state = component["state"]
        if state["count"] != count:
            raise RuntimeError("incomplete integral sample count")
        mean = state["sum_weighted_value"] / count
        stderr = math.sqrt(max(0., state["sum_sq"] / count - mean * mean) / (count - 1))
        integral[component["name"]] = dict(mean=mean, stderr=stderr)
    diagnostics = output.get("diagnostics", {})
    coverage = diagnostics.get("count_total", 0)
    if coverage not in (0, count):
        raise RuntimeError(f"{phase}: partial diagnostic coverage {coverage}/{count}")
    return dict(
        diagnostics_samples=coverage,
        diagnostic_counters={k:v for k,v in diagnostics.items() if not k.endswith("_ms")},
        integral=integral, histogram_count=len(histograms),
        histogram_sha256=hashlib.sha256(json.dumps(histograms, sort_keys=True, allow_nan=False).encode()).hexdigest(),
    )


def run_case(args, backend, label, params, domain, samples, cpus):
    output = args.output / label
    output.mkdir()
    run_card = output / "run.toml"
    run_card.write_text(card(backend, params, domain, args.worker, samples, args.batch_size, args.observable_format))
    started = time.monotonic()
    before_load = common.core_loads(cpus)
    print(f"{label}: starting fresh instance; samples={samples}", flush=True)
    with common.Session(
        args.binary, output, budget=1800, offset=args.port_offset,
        infrastructure_cpus=cpus[:2], shared_buffers="128MB",
    ) as session:
        shutil.copy2(session.instance, output / "instance.toml")
        session.start_pinned_workers([cpus[3]], cpus[2])
        workers = session.cli("worker", "list")
        common.write_json(output / "workers.json", workers)
        instance_ready = time.monotonic()
        created = session.cli("run", "create", run_card, timeout=180)
        run_id = created["run_id"]
        common.write_json(output / "created.json", created)
        session.cli("run", "start", run_id, "--max-evaluators", 1)
        seen = set()
        deadline = time.monotonic() + 1200
        while time.monotonic() < deadline:
            tasks = session.cli("run", "task", "list", run_id)
            for task in tasks:
                if task["state"] == "failed":
                    raise RuntimeError(f"{label}: {task['name']} failed: {task['failure_reason']}")
                if task["state"] == "completed" and task["name"] not in seen:
                    seen.add(task["name"])
                    duration = seconds(task["started_at"], task["completed_at"])
                    print(f"{label}/{task['name']}: {duration:.3f}s, {task['nr_completed_samples']} samples", flush=True)
            if len(tasks) == 2 and len(seen) == 2:
                break
            time.sleep(0.5)
        else:
            raise TimeoutError(f"{label} did not complete within 20 minutes")
        inspect = session.cli("run", "inspect", run_id, "--log-limit", 100)
        results = session.cli("run", "results", run_id)
        common.write_json(output / "inspect.json", inspect)
        common.write_json(output / "results.json", results)
        common.write_json(output / "status.json", session.cli("status"))
        log_dir = session.directory / "logs"
        if log_dir.exists():
            shutil.copytree(log_dir, output / "service-logs")
        rows = []
        for task in tasks:
            phase = task["name"]
            count = task["nr_completed_samples"]
            if count != samples[phase]:
                raise RuntimeError(f"{label}/{phase}: expected {samples[phase]} samples, got {count}")
            duration = seconds(task["started_at"], task["completed_at"])
            if duration <= 0:
                raise RuntimeError("invalid task timestamps")
            rows.append(dict(
                case=label, backend=backend, phase=phase, samples=count,
                seconds=duration, samples_per_second=count / duration,
                task_id=task["id"], started_at=task["started_at"], completed_at=task["completed_at"],
                reported_cpu_seconds=task["cpu_seconds"],
                **scientific_result(results, phase, count),
            ))
    common.write_json(output / "timing.json", dict(
        setup_seconds=instance_ready - started,
        total_seconds_including_setup_and_cleanup=time.monotonic() - started,
        core_load_before=before_load, core_load_after=common.core_loads(cpus), rows=rows,
    ))
    return rows


def summarize(output, rows, metadata):
    columns = ["case", "backend", "phase", "samples", "seconds", "samples_per_second", "reported_cpu_seconds"]
    with (output / "measurements.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=columns, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    summary = []
    for phase in ("train", "inference"):
        native_mean = statistics.mean(r["seconds"] for r in rows if r["phase"] == phase and r["backend"] == "native")
        for backend in ("native", "process"):
            selected = [r for r in rows if r["phase"] == phase and r["backend"] == backend]
            durations = [r["seconds"] for r in selected]
            mean = statistics.mean(durations)
            summary.append(dict(
                phase=phase, backend=backend, samples=selected[0]["samples"], trials=len(selected),
                mean_seconds=mean, min_seconds=min(durations), max_seconds=max(durations),
                stdev_seconds=statistics.stdev(durations) if len(durations) > 1 else None,
                samples_per_second=selected[0]["samples"] / mean,
                time_ratio_to_native=mean / native_mean,
            ))
    checks = []
    for phase in ("train", "inference"):
        native = [r for r in rows if r["phase"] == phase and r["backend"] == "native"]
        process = [r for r in rows if r["phase"] == phase and r["backend"] == "process"]
        for index, (a, b) in enumerate(zip(native, process), 1):
            if "integral" in a:
                checks.append(dict(
                    phase=phase, pair=index, native_integral=a["integral"], process_integral=b["integral"],
                    integral_equal=a["integral"] == b["integral"],
                    histograms_equal=a["histogram_sha256"] == b["histogram_sha256"],
                    histogram_count=a["histogram_count"],
                    diagnostics_complete=a.get("diagnostics_samples")==a["samples"] and b.get("diagnostics_samples")==b["samples"],
                    diagnostic_counters_equal=a.get("diagnostic_counters")==b.get("diagnostic_counters"),
                ))
    common.write_json(output / "scientific-checks.json", checks)
    common.write_json(output / "summary.json", dict(metadata=metadata, summary=summary, measurements=rows, scientific_checks=checks))
    table = "\n".join(
        f"<tr><td>{r['phase']}</td><td>{r['backend']}</td><td>{r['samples']:,}</td>"
        f"<td>{r['mean_seconds']:.3f}</td><td>{r['min_seconds']:.3f}–{r['max_seconds']:.3f}</td>"
        f"<td>{r['samples_per_second']:,.0f}</td><td>{r['time_ratio_to_native']:.3f}×</td></tr>"
        for r in summary
    )
    (output / "report.html").write_text(f'''<!doctype html><meta charset="utf-8">
<title>GammaLoop native / process A/B</title>
<style>body{{font:16px system-ui;max-width:1100px;margin:3em auto;padding:0 2em;color:#223}}
table{{border-collapse:collapse;width:100%}}td,th{{padding:.7em;text-align:right;border-bottom:1px solid #ccd}}
td:first-child,td:nth-child(2){{text-align:left}}pre{{white-space:pre-wrap;background:#f3f5f8;padding:1em}}</style>
<h1>GammaLoop: native versus process API</h1>
<p>Fresh real PostgreSQL/service deployment per trial. One pinned evaluator and one
separate pinned sampler; the process child shares the evaluator core. Same saved
state, seeds, fixed sample budgets, batch sizes and observable accumulators.
Havana training is followed by inference from its trained grid.</p>
<table><tr><th>Stage</th><th>Backend</th><th>Samples</th><th>Mean seconds</th><th>Range</th><th>Samples/s</th><th>Time/native</th></tr>{table}</table>
<p>Stage time is the database task's completed_at minus started_at: it includes
task initialization, sampling, evaluation, IPC, database traffic, reduction and
finalization. Instance setup/teardown and run creation are excluded and retained
separately in each timing.json. Calibration trials are excluded. CPU cores are
pinned but the host is shared; the observed range is not a confidence interval.
Thirty-second timing windows do not imply high statistical accuracy of the integral.</p>
<p><a href="measurements.csv">All measurements (CSV)</a> · <a href="summary.json">Summary (JSON)</a></p>
<h2>Numerical and observable checks</h2>
<p>Every saved histogram must contain the full sample count. These comparisons
compare diagnostic counters and coverage as well as numerical results. Measured
diagnostic timings are checked separately, since elapsed times cannot match exactly. Errors below are Monte Carlo standard errors, not timing errors.</p>
<pre>{html.escape(json.dumps(checks, indent=2))}</pre>
<h2>Reproduction metadata</h2><pre>{html.escape(json.dumps(metadata, indent=2))}</pre>''')
    return summary


def available_offset():
    for offset in range(18000, 19000):
        try:
            with socket.socket() as api, socket.socket() as db:
                api.bind(("127.0.0.1", 4000 + offset))
                db.bind(("127.0.0.1", 5400 + offset))
                return offset
        except OSError:
            pass
    raise RuntimeError("no available benchmark port pair")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="optimized gammaboard binary; gb must be beside it")
    parser.add_argument("--worker", type=Path, required=True, help="matching optimized gammaloop_process binary")
    parser.add_argument("--args", type=Path, required=True, help="GammaLoop parameter JSON, as for wrapper --describe")
    parser.add_argument("--output", type=Path, required=True, help="fresh output directory")
    parser.add_argument("--seconds", type=float, default=30, help="approximate target per stage")
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--batch-size", type=int, default=512, choices=[128, 256, 512, 1024, 2048])
    parser.add_argument("--cpus", help="four distinct physical cores: two infrastructure, sampler, evaluator")
    parser.add_argument("--port-offset", type=int)
    parser.add_argument("--train-samples", type=int, help="fixed training budget; skips calibration when both budgets are set")
    parser.add_argument("--inference-samples", type=int)
    parser.add_argument("--observable-format", choices=["binary", "json"], help="override the Rust wrapper's observable transport")
    args = parser.parse_args()
    if not math.isfinite(args.seconds) or args.seconds < 5 or args.repetitions < 1:
        parser.error("seconds must be finite and >= 5; repetitions must be positive")
    if (args.train_samples is None) != (args.inference_samples is None):
        parser.error("provide both --train-samples and --inference-samples")
    if args.train_samples is not None and min(args.train_samples, args.inference_samples) < 2:
        parser.error("fixed sample budgets must be at least 2")
    args.binary, args.worker, args.output = args.binary.resolve(), args.worker.resolve(), args.output.resolve()
    params = json.loads(args.args.read_text())
    state = Path(params["state_folder"])
    if not state.is_absolute() or not (state / "state_manifest.toml").is_file():
        parser.error("args.state_folder must name an absolute saved-state directory")
    if not params.get("preprocessing", {}).get("read_only", True):
        parser.error("benchmark preprocessing must be read-only")
    cpus = list(map(int, args.cpus.split(","))) if args.cpus else common.idle_cpus(common.physical_cpus(), 4)
    if len(set(cpus)) != 4 or not set(cpus) <= set(common.physical_cpus()):
        parser.error("four available, distinct physical cores are required")
    args.port_offset = args.port_offset if args.port_offset is not None else available_offset()
    args.output.mkdir(parents=True, exist_ok=False)
    inputs = args.output / "inputs"
    inputs.mkdir()
    for source, name in [(args.binary, "gammaboard"), (args.binary.with_name("gb"), "gb"), (args.worker, "gammaloop_process")]:
        shutil.copy2(source, inputs / name)
    args.binary, args.worker = inputs / "gammaboard", inputs / "gammaloop_process"
    shutil.copytree(state, inputs / "state")
    shutil.copytree(common.ROOT / "migrations", inputs / "migrations")
    shutil.copytree(common.ROOT / "benchmarks", inputs / "benchmarks", ignore=shutil.ignore_patterns("__pycache__"))
    import os
    os.environ["GAMMABOARD_MIGRATIONS_DIR"] = str(inputs / "migrations")
    params["state_folder"] = str(inputs / "state")
    common.write_json(inputs / "args.json", params)
    description = subprocess.run([str(args.worker), "--describe", str(inputs / "args.json")], capture_output=True, text=True, timeout=180, check=True)
    (inputs / "describe.stderr").write_text(description.stderr)
    description = json.loads(description.stdout)
    common.write_json(inputs / "description.json", description)
    hashes = state_hashes(inputs / "state")
    common.write_json(inputs / "state-hashes.json", hashes)
    metadata = dict(
        **common.machine_metadata(), seconds_target=args.seconds, repetitions=args.repetitions,
        cpus=dict(infrastructure=cpus[:2], sampler=cpus[2], evaluator=cpus[3]),
        batch_size=args.batch_size, training_seed=41, inference_seed=42,
        build_info=json.loads(subprocess.check_output([str(args.binary), "build-info"], text=True)),
        binary_hashes={name: common.file_hash(inputs / name) for name in ("gammaboard", "gb", "gammaloop_process")},
        source_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=common.ROOT, text=True).strip(),
        source_dirty=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=common.ROOT, text=True)),
    )
    common.write_json(args.output / "manifest.json", metadata)
    if args.train_samples is not None:
        samples = dict(train=args.train_samples, inference=args.inference_samples)
    else:
        samples = dict(train=8192, inference=8192)
        for attempt in range(3):
            pilot = []
            for backend in ("native", "process"):
                pilot.extend(run_case(args, backend, f"calibration-{attempt}-{backend}", params, description["domain"], samples, cpus))
            shortest = {phase: min(r["seconds"] for r in pilot if r["phase"] == phase) for phase in samples}
            if min(shortest.values()) >= 5 or attempt == 2:
                break
            samples = {phase: max(n, math.ceil(n * 6 / shortest[phase] / 2048) * 2048) for phase, n in samples.items()}
        samples = {
            phase: max(4096, round(args.seconds * statistics.geometric_mean(
                r["samples_per_second"] for r in pilot if r["phase"] == phase
            ) / 2048) * 2048) for phase in samples
        }
    metadata["calibrated"] = args.train_samples is None
    metadata["observable_format_override"] = args.observable_format
    metadata["samples"] = samples
    common.write_json(args.output / "manifest.json", metadata)
    rows = []
    for repeat in range(args.repetitions):
        for backend in (("native", "process") if repeat % 2 == 0 else ("process", "native")):
            rows.extend(run_case(args, backend, f"trial-{repeat+1}-{backend}", params, description["domain"], samples, cpus))
            common.write_json(args.output / "measurements.json", rows)
    metadata["state_unchanged"] = hashes == state_hashes(inputs / "state")
    common.write_json(args.output / "manifest.json", metadata)
    if not metadata["state_unchanged"]:
        raise RuntimeError("state changed during measurements; inspect retained artifacts")
    summary = summarize(args.output, rows, metadata)
    print(json.dumps(summary, indent=2), flush=True)
    print(f"Report: {args.output / 'report.html'}", flush=True)


if __name__ == "__main__":
    main()
