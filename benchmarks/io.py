"""Separate sampler/evaluator database-path capacity, using the Rust helper."""

import json
import math
import os
import statistics
import sys
import time

from . import common as bench
from .reporting import publish, figure_save, batch_axis


def assess(row):
    issues = []
    for name in ("samples_per_second", "batches_per_second", "elapsed_seconds"):
        if (
            not isinstance(row.get(name), (int, float))
            or not math.isfinite(row[name])
            or row[name] <= 0
        ):
            issues.append(f"invalid {name}")
    busy = row.get("measured_io_busy")
    if not isinstance(busy, (int, float)) or not math.isfinite(busy) or not 0 <= busy <= 1.001:
        issues.append("invalid busy window")
    if row.get("batches", 0) < 8:
        issues.append("fewer than eight completed batches")
    if row.get("role") == "evaluator" and any(w["empty_claims"] for w in row.get("windows", [])):
        issues.append("prefilled queue starved")
    row["valid"] = not issues
    row["issues"] = issues
    row["saturated"] = row["valid"] and busy > 0.90
    if row["valid"] and not row["saturated"]:
        row["issues"].append(
            "I/O busy is not above 90%; inspect consumer capacity, buffering and CPU scheduling"
        )
    return row


def thread_counts(requested, core_count):
    """Keep two cores for the database and consumers; explicit sweeps must fit."""
    if requested is None:
        requested = [n for n in (1, 2, 4, 8) if n + 2 <= core_count]
    if (
        not requested
        or len(set(requested)) != len(requested)
        or any(not 1 <= n <= 32 for n in requested)
    ):
        raise ValueError(
            "I/O threads must be unique values in 1..32, with at least three physical cores available"
        )
    if max(requested) + 2 > core_count:
        raise ValueError("need separate sampler, database and evaluator cores")
    return sorted(requested)


def execute(args):
    sizes = args.batch_sizes or [256, 4096, 16384, 65536, 131072]
    if len(set(sizes)) != len(sizes) or any(not 16 <= n <= 1048576 for n in sizes):
        raise ValueError("batch sizes must be unique and between 16 and 1048576")
    if max(sizes) * 160 * 8 > args.memory_mib * 1024**2:
        raise ValueError(
            "memory budget cannot hold eight largest batches; increase --memory-mib or reduce --batch-sizes"
        )
    if not 0.5 <= args.duration <= 60 or not 0.1 <= args.warmup <= 10:
        raise ValueError("duration must be 0.5..60s and warmup 0.1..10s")
    if not 1 <= args.repetitions <= 10 or not 1 <= args.consumers <= 64:
        raise ValueError("invalid repetition or consumer count")
    if (
        not 64 <= args.database_cache_mib <= 16384
        or not 1 <= args.insert_concurrency <= 16
        or not 8 <= args.queue_batches <= 512
    ):
        raise ValueError("invalid database cache, insert concurrency or queue capacity")
    role = args.command.removesuffix("-io")
    candidates = bench.physical_cpus()
    available = min(len(candidates), args.cpu_limit or len(candidates))
    threads = thread_counts(args.io_threads if role == "sampler" else [1], available)
    count = min(available, max(threads) + (args.database_cores or 12) + 16)
    cpus = bench.idle_cpus(candidates, count)
    sampler_cpus = cpus[: max(threads)]
    db_count = args.database_cores or min(15, max(1, (count - len(sampler_cpus)) // 2))
    if db_count < 1 or db_count + len(sampler_cpus) >= count:
        raise ValueError("database CPU allocation must leave a consumer core")
    database_cpus = cpus[len(sampler_cpus) : len(sampler_cpus) + db_count]
    evaluator_cpus = cpus[len(sampler_cpus) + db_count :]
    if role == "evaluator":
        evaluator_cpus = evaluator_cpus[:1]
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    binary, hashes = bench.preserve_inputs(output, args.binary.resolve(strict=True))
    cases = [(size, feedback, n) for size in sizes for feedback in [False, True] for n in threads]
    started = time.monotonic()
    manifest = dict(
        experiment="io",
        role=role,
        schema_version=1,
        status="running",
        binary_sha256=bench.file_hash(binary),
        harness_files=hashes,
        batch_sizes=sizes,
        repetitions=args.repetitions,
        io_threads=threads,
        sampler_cpus=sampler_cpus,
        database_cpus=database_cpus,
        evaluator_cpus=evaluator_cpus,
        memory_mib=args.memory_mib,
        database_cache_mib=args.database_cache_mib,
        insert_concurrency=args.insert_concurrency,
        queue_batches=args.queue_batches,
        profile=args.profile,
        database_directory=str(args.database_directory),
        planned_cases=len(cases) * args.repetitions,
        load_average=os.getloadavg(),
        elapsed_seconds=0,
        scope="Production storage capacity with prepared 6D inputs and f(x)=x[0] results; training means feedback transport, without model updates.",
    )
    manifest.update(bench.machine_metadata())
    bench.write_json(output / "manifest.json", manifest)
    records = []
    try:
        with bench.Session(
            binary,
            output,
            args.budget or 1800,
            args.port_offset,
            max_connections=2 * args.consumers + args.insert_concurrency + 32,
            infrastructure_cpus=database_cpus,
            database_directory=args.database_directory,
            shared_buffers=f"{args.database_cache_mib}MB",
        ) as session:
            for repeat in range(args.repetitions):
                for size, feedback, n in (cases if repeat % 2 == 0 else list(reversed(cases))):
                    config = dict(
                        role=role,
                        batch_size=size,
                        feedback=feedback,
                        io_threads=n,
                        duration_seconds=args.duration,
                        warmup_seconds=args.warmup,
                        consumers=args.consumers if role == "sampler" else 1,
                        memory_mib=args.memory_mib,
                        insert_concurrency=args.insert_concurrency,
                        queue_batches=args.queue_batches,
                        profile=args.profile,
                        sampler_cpus=sampler_cpus,
                        evaluator_cpus=evaluator_cpus,
                    )
                    directory = output / f"case-{len(records):03d}"
                    directory.mkdir()
                    bench.write_json(directory / "config.json", config)
                    row = session.cli("benchmark", "io", directory / "config.json", timeout=300)
                    row = assess(dict(row, repeat=repeat, case=directory.name))
                    records.append(row)
                    bench.write_json(directory / "measurement.json", row)
                    with (output / "results.jsonl").open("a") as stream:
                        stream.write(json.dumps(row, allow_nan=False) + "\n")
                    print(
                        f"{role} {size:,}/batch feedback={feedback} io_threads={n}: "
                        f'{row["samples_per_second"]/1e6:.3f} M/s',
                        flush=True,
                    )
                    for issue in row["issues"]:
                        print(
                            f"WARNING: {directory.name}: {issue} "
                            f'(measured busy {row["measured_io_busy"]:.1%})',
                            file=sys.stderr,
                            flush=True,
                        )
        manifest["status"] = (
            "incomplete"
            if any(not r["valid"] for r in records)
            else "completed" if all(r["saturated"] for r in records) else "completed_with_flags"
        )
    except BaseException as error:
        manifest.update(status="incomplete", error=str(error) or type(error).__name__)
        raise
    finally:
        manifest.update(completed_cases=len(records), elapsed_seconds=time.monotonic() - started)
        bench.write_json(output / "manifest.json", manifest)
    return output


def summarize(records):
    groups = {}
    seen = set()
    for row in records:
        key = (row["batch_size"], row["feedback"], row["io_threads"])
        identity = (*key, row["repeat"])
        if identity in seen:
            raise ValueError("duplicate trial")
        seen.add(identity)
        groups.setdefault(key, []).append(row)
    rows = []
    for (batch, feedback, threads), group in sorted(groups.items()):
        valid = [r for r in group if r["valid"]]
        med = lambda key: statistics.median(r[key] for r in valid) if valid else None
        rate = med("samples_per_second")
        busy = med("measured_io_busy")
        rows.append(
            dict(
                batch_size=batch,
                feedback=feedback,
                io_threads=threads,
                samples_per_second=rate,
                batches_per_second=med("batches_per_second"),
                partner_io_busy_percent=(
                    100 * med("evaluator_io_busy")
                    if valid and group[0]["role"] == "sampler"
                    else None
                ),
                io_busy_percent=100 * busy if busy is not None else None,
                input_MiB_per_second=(
                    rate / batch * med("input_bytes_per_batch") / 1024**2 if rate else None
                ),
                feedback_MiB_per_second=(
                    rate / batch * med("feedback_bytes_per_batch") / 1024**2 if rate else None
                ),
                queue_slots=med("queue_slots"),
                insert_bundle=med("insert_bundle"),
                valid_trials=len(valid),
                flagged_trials=sum(not r["saturated"] for r in group),
            )
        )
    return rows


def report(directory):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    manifest = json.loads((directory / "manifest.json").read_text())
    records = [assess(row) for row in bench.load_results(directory)]
    rows = summarize(records)
    bench.write_json(directory / "summary.json", rows)
    # Feedback modes share a scale; colors identify thread counts in both panels.
    fig, axes = plt.subplots(1, 2, figsize=(12, 4.5), sharex=True, sharey=True)
    for ax, feedback in zip(axes, [False, True]):
        for index, threads in enumerate(manifest["io_threads"]):
            group = [
                r
                for r in rows
                if r["feedback"] == feedback
                and r["io_threads"] == threads
                and r["samples_per_second"] is not None
            ]
            if not group:
                continue
            label = f"{threads} I/O thread" + ("s" if threads != 1 else "")
            ax.plot(
                [r["batch_size"] for r in group],
                [r["samples_per_second"] / 1e6 for r in group],
                marker="o",
                color=f"C{index}",
                label=label,
            )
            if manifest["repetitions"] > 1:
                trials = [
                    r
                    for r in records
                    if r["feedback"] == feedback and r["io_threads"] == threads and r["valid"]
                ]
                ax.scatter(
                    [r["batch_size"] for r in trials],
                    [r["samples_per_second"] / 1e6 for r in trials],
                    color=f"C{index}",
                    s=14,
                    alpha=0.35,
                    zorder=3,
                )
        ax.set_title("With training feedback" if feedback else "Without training feedback")
        batch_axis(ax, manifest["batch_sizes"])
        ax.grid(alpha=0.2)
        if ax.lines:
            ax.legend(fontsize=9)
    axes[0].set_ylabel("Million samples/s")
    axes[0].set_ylim(bottom=0)
    fig.suptitle(
        f'{manifest["role"].title()} database I/O capacity · prepared materialized samples'
    )
    fig.tight_layout()
    figure_save(fig, directory, "throughput")
    plt.close(fig)
    notes = [
        manifest["scope"],
        "Lines show medians; small faint dots show individual repeats when available. Short shared-host trials show variation, not confidence bounds. Repeats reverse case order in the same private database; cache state and background database work can affect rates.",
        "Busy uses the production occupied-wall-time timer. Concurrent operations count once; completed operations awaiting collection do not count. Database waits do count.",
        "Busy at or below 90% produces a warning; rates remain visible. Busy and consumer activity are retained in the data, not plotted. High busy alone does not establish a hardware ceiling.",
        "Evaluator passes are prefilled and warmed, with no producer during timing. Preparation and cleanup between passes are excluded. Sampler throughput counts collected results and includes ongoing publication and cleanup.",
        "Payload MiB/s counts encoded inputs/feedback, not network, WAL or physical disk traffic. Repeated inputs vary within each batch; feedback is f(x)=x[0].",
        "All thread counts use the same role CPU sets and memory budget. That budget can reduce queue slots and insert bundle size for large batches; effective values are included in the table.",
    ]
    return publish(directory, f'{manifest["role"].title()} I/O', rows, ["throughput.png"], notes)
