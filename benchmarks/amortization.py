"""Matched native CPU workload: direct memory pipeline versus production runners."""

import json
import os
import statistics
import subprocess
import time
import tomllib

from . import common as bench, frontier
from .reporting import figure_save, publish

BATCH_SIZES = [128, 512, 2048, 8192, 32768]
WORKERS = [1, 16]
GENERATION_SIZE = 131072


def run_direct(binary, directory, config):
    directory.mkdir()
    bench.write_json(directory / "config.json", config)
    result = subprocess.run(
        [str(binary), "--json", "benchmark", "amortization", str(directory / "config.json")],
        cwd=bench.ROOT,
        capture_output=True,
        text=True,
        timeout=180,
    )
    (directory / "stderr.log").write_text(result.stderr)
    if result.returncode:
        raise RuntimeError(f"direct reference failed: {result.stderr}")
    measured = json.loads(result.stdout)
    bench.write_json(directory / "measurement.json", measured)
    return measured


def comparison(direct, runner):
    if not runner["valid"] or not runner["adequate"] or direct["rate"] <= 0:
        raise ValueError("invalid amortization measurement")
    return dict(
        direct_samples_per_second=direct["rate"],
        gammaboard_samples_per_second=runner["rate"],
        evaluator_batch_ms=1000 * direct["mean_evaluate_batch_seconds"],
        overhead_percent=100 * (direct["rate"] / runner["rate"] - 1),
    )


def execute(args):
    workers = args.workers or WORKERS
    sizes = args.batch_sizes or BATCH_SIZES
    if len(workers) != len(set(workers)) or any(n < 1 or n > 64 for n in workers):
        raise ValueError("amortization worker counts must be unique and in 1..64")
    if len(sizes) != len(set(sizes)) or any(
        n < 16 or n > 32768 or GENERATION_SIZE % n for n in sizes
    ):
        raise ValueError("batch sizes must divide 131072 and be in 16..32768")
    candidates = bench.physical_cpus()
    required = max(workers) + 9
    if len(candidates) < required or (args.cpu_limit and args.cpu_limit < required):
        raise ValueError(f"amortization needs {required} physical cores for these worker counts")
    loads = bench.core_loads(candidates, seconds=2)
    cpus = sorted(sorted(candidates, key=lambda cpu: (loads[cpu], cpu))[:required])
    sampler_cpus, database_cpus, evaluator_cpus = cpus[:5], cpus[5:9], cpus[9:]
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    binary, hashes = bench.preserve_inputs(output, args.binary)
    for key in (
        "OMP_NUM_THREADS",
        "OPENBLAS_NUM_THREADS",
        "MKL_NUM_THREADS",
        "TOKIO_WORKER_THREADS",
    ):
        os.environ[key] = "1"
    manifest = dict(
        experiment="amortization",
        schema_version=1,
        status="running",
        workers=workers,
        batch_sizes=sizes,
        repetitions=args.repetitions,
        duration_seconds=args.duration,
        generation_size=GENERATION_SIZE,
        initial_core_loads={cpu: loads[cpu] for cpu in cpus},
        sampler_cpus=sampler_cpus,
        database_cpus=database_cpus,
        evaluator_cpus=evaluator_cpus,
        binary_sha256=bench.file_hash(binary),
        harness_files=hashes,
        scope="Native CPU work, six uniform coordinates, f(x)=x[0]; same sampler/evaluator/accumulator engines and generation-level feedback. Direct bounded in-memory pipeline versus PostgreSQL and production runners. Steady-state accepted throughput; startup/final drain excluded. Shared host; feedback is transport/ingestion, not model optimization.",
        elapsed_seconds=0,
        **bench.machine_metadata(),
    )
    bench.write_json(output / "manifest.json", manifest)
    start = time.monotonic()
    budget = args.budget or 3600
    config = dict(
        batch_size=1024,
        generation_size=16384,
        cpu_iterations_per_sample=2000,
        feedback=False,
        duration_seconds=0.3,
        evaluator_cpus=evaluator_cpus[:1],
        sampler_cpus=sampler_cpus,
    )
    try:
        pilot = run_direct(binary, output / "calibration", config)
        cost = pilot["mean_evaluate_batch_seconds"] / config["batch_size"]
        iterations = max(1, round(config["cpu_iterations_per_sample"] * 5e-6 / cost))
        config.update(
            cpu_iterations_per_sample=iterations,
            generation_size=GENERATION_SIZE,
            duration_seconds=args.duration,
        )
        manifest.update(cpu_iterations_per_sample=iterations, calibration=pilot)
        bench.write_json(output / "manifest.json", manifest)
        suite = tomllib.loads((bench.ROOT / "benchmarks/frontier.toml").read_text())
        suite.update(
            workers=workers,
            generation_batch_size=GENERATION_SIZE,
            sampler_io_threads=4,
            database_shared_buffers="1GB",
            measurement_seconds=args.duration,
        )
        records = []
        for repeat in range(args.repetitions):
            for count in (workers if repeat % 2 == 0 else workers[::-1]):
                directory = output / f"repeat-{repeat:02}-workers-{count}"
                directory.mkdir()
                remaining = budget - (time.monotonic() - start)
                if remaining < 180:
                    raise TimeoutError("amortization time budget exhausted")
                # A fresh database/fleet per count and repeat, paired direct/runner cases.
                with bench.Session(
                    binary,
                    directory,
                    remaining,
                    args.port_offset,
                    infrastructure_cpus=database_cpus,
                    shared_buffers="1GB",
                ) as session:
                    session.start_pinned_workers(evaluator_cpus[:count], sampler_cpus)
                    selected = sizes if repeat % 2 == 0 else sizes[::-1]
                    modes = (
                        ["materialized", "training"]
                        if repeat % 2 == 0
                        else ["training", "materialized"]
                    )
                    for size in selected:
                        for mode in modes:
                            case = directory / f"batch-{size}-{mode}"
                            case.mkdir()
                            point = frontier.Point(mode, 5.0, count, size)
                            # Small batches are tick-limited, so nominal CPU time
                            # alone cannot guarantee full feedback-cycle coverage.
                            case_suite = dict(
                                suite,
                                measurement_seconds=max(
                                    args.duration,
                                    2 * GENERATION_SIZE * max(5e-6, 0.012 / size) / count,
                                ),
                            )
                            settings = frontier.queue_settings(point, suite)
                            text = frontier.card(mode, 0, suite).replace(
                                "cpu_iterations_per_sample = 0",
                                f"cpu_iterations_per_sample = {iterations}",
                            )
                            native = dict(
                                config,
                                batch_size=size,
                                feedback=mode == "training",
                                evaluator_cpus=evaluator_cpus[:count],
                            )
                            results = {}
                            for role in (
                                ["direct", "gammaboard"]
                                if repeat % 2 == 0
                                else ["gammaboard", "direct"]
                            ):
                                if role == "direct":
                                    results[role] = run_direct(binary, case / role, native)
                                else:
                                    (case / "window").mkdir()
                                    with frontier.LiveRun(
                                        session,
                                        case / role,
                                        case_suite,
                                        mode,
                                        5.0,
                                        point,
                                        card_text=text,
                                    ) as live:
                                        results[role] = live.measure(point, case / "window")
                            row = dict(
                                repeat=repeat,
                                workers=count,
                                batch=size,
                                mode=mode,
                                directory=str(case.relative_to(output)),
                                load_average=os.getloadavg(),
                                settings=settings,
                                **comparison(results["direct"], results["gammaboard"]),
                            )
                            records.append(row)
                            bench.write_json(output / "results.json", records)
                            print(
                                f"{len(records)}/{args.repetitions*len(workers)*len(sizes)*2}: "
                                f"N={count} B={size} {mode}: {row['overhead_percent']:.1f}% "
                                f"overhead, compute {row['evaluator_batch_ms']:.3f} ms",
                                flush=True,
                            )
        manifest["status"] = "completed"
    except BaseException:
        manifest["status"] = "incomplete"
        raise
    finally:
        manifest["elapsed_seconds"] = time.monotonic() - start
        bench.write_json(output / "manifest.json", manifest)
    return output


def summarize(records):
    rows = []
    for workers, mode, batch in sorted({(r["workers"], r["mode"], r["batch"]) for r in records}):
        group = [
            r for r in records if (r["workers"], r["mode"], r["batch"]) == (workers, mode, batch)
        ]
        values = [r["overhead_percent"] for r in group]
        rows.append(
            dict(
                workers=workers,
                mode=mode,
                batch=batch,
                trials=len(group),
                evaluator_batch_ms=statistics.median(r["evaluator_batch_ms"] for r in group),
                overhead_percent=statistics.median(values),
                overhead_low=min(values),
                overhead_high=max(values),
                direct_samples_per_second=statistics.median(
                    r["direct_samples_per_second"] for r in group
                ),
                gammaboard_samples_per_second=statistics.median(
                    r["gammaboard_samples_per_second"] for r in group
                ),
            )
        )
    return rows


def report(directory):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    records = (
        json.loads((directory / "results.json").read_text())
        if (directory / "results.json").exists()
        else []
    )
    rows = summarize(records)
    bench.write_json(directory / "summary.json", rows)
    fig, axes = plt.subplots(1, 2, figsize=(12, 5.0), sharey=True)
    for ax, mode in zip(axes, ["materialized", "training"]):
        for count, color in zip(sorted({r["workers"] for r in rows}), ["#2563eb", "#c2410c"]):
            group = [r for r in rows if r["mode"] == mode and r["workers"] == count]
            x = [r["evaluator_batch_ms"] for r in group]
            y = [r["overhead_percent"] for r in group]
            ax.plot(
                x, y, "o-", color=color, label=f"{count} evaluator" + ("s" if count != 1 else "")
            )
            for point in group:
                dots = [
                    r
                    for r in records
                    if r["mode"] == mode and r["workers"] == count and r["batch"] == point["batch"]
                ]
                ax.scatter(
                    [r["evaluator_batch_ms"] for r in dots],
                    [r["overhead_percent"] for r in dots],
                    color=color,
                    alpha=0.3,
                    s=18,
                )
            for a, b, row in zip(x, y, group):
                label = f"{row['batch']//1024}k" if row["batch"] >= 1024 else str(row["batch"])
                ax.annotate(
                    label,
                    (a, b),
                    xytext=(4, 6 if count == 1 else -12),
                    textcoords="offset points",
                    fontsize=8,
                    color=color,
                )
        ax.axhline(5, color="#64748b", linestyle="--", linewidth=1, label="5% overhead")
        ax.axhline(0, color="#94a3b8", linewidth=0.7)
        if not any(r["mode"] == mode for r in rows):
            ax.set_xlim(0.1, 1000)
        ax.set_xscale("log")
        ax.set_yscale("symlog", linthresh=10)
        ax.set_xlabel("Direct evaluator time per batch (ms)")
        ax.set_title("Feedback off" if mode == "materialized" else "Feedback on")
        ax.grid(alpha=0.2)
        ax.legend(fontsize=8)
    axes[0].set_ylabel("Extra steady-state runtime (%)")
    fig.suptitle("GammaBoard amortization · matched CPU workload")
    fig.text(
        0.5,
        0.015,
        "Labels: samples/batch (k = 1,024). Lines: median paired overhead; dots: independent runs.\nReference includes native sampling, evaluation and accumulation; excludes database/runners. Shared host.\nY axis: linear within ±10%, logarithmic beyond.",
        ha="center",
        fontsize=8,
    )
    fig.tight_layout(rect=(0, 0.10, 1, 1))
    figure_save(fig, directory, "amortization")
    fig.savefig(directory / "amortization.pdf", bbox_inches="tight")
    plt.close(fig)
    return publish(
        directory,
        "GammaBoard amortization",
        rows,
        ["amortization.png"],
        [
            "Extra runtime = 100 × (direct accepted throughput / GammaBoard accepted throughput − 1), an equivalent steady-state work comparison. Startup and final draining are excluded.",
            "Same fixed CPU arithmetic, six-dimensional uniform sampler, scalar accumulation, generation size and optional generation feedback. Native engines; this is not a process-API or GLNIS optimizer measurement.",
            "The x axis is measured direct evaluator call time, including local accumulation. The y axis is linear near zero and logarithmic beyond ±10%. Negative values mean the production pipeline was faster in that pair; they are not clipped.",
            "Fixed disjoint core allocations, not an exclusive host. Run-to-run range is saved in CSV; plotted dots are repetitions, not confidence intervals.",
        ],
    )
