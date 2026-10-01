"""Paired Rust/Python adapter timings; runnable from the normal optimized binary."""

import json
import math
import os
import shutil
import statistics
import subprocess
import time

from . import common as bench
from .reporting import publish, figure_save, batch_axis


def execute(args):
    sizes = args.batch_sizes or [16, 64, 256, 1024, 4096, 16384, 32768, 131072]
    if len(set(sizes)) != len(sizes) or any(not 16 <= n <= 1048576 for n in sizes):
        raise ValueError("protocol batch sizes must be unique and in 16..1048576")
    cpus = bench.idle_cpus(bench.physical_cpus(), 2)
    if len(cpus) != 2:
        raise ValueError("protocol measurement requires two physical cores")
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    binary, hashes = bench.preserve_inputs(output, args.binary.resolve(strict=True))
    # The copied fixture preserves its relative SDK import; no build-time paths.
    source = output / "process_api/python"
    shutil.copytree(
        bench.ROOT / "process_api/python/src",
        source / "src",
        ignore=shutil.ignore_patterns("__pycache__"),
    )
    (source / "tests").mkdir()
    shutil.copy2(
        bench.ROOT / "process_api/python/tests/runtime_fixture.py",
        source / "tests/runtime_fixture.py",
    )
    config = dict(
        python=str(args.python.resolve()),
        fixture=str(source / "tests/runtime_fixture.py"),
        output=str(output),
        batch_sizes=sizes,
        child_cpu=cpus[1],
    )
    bench.write_json(output / "config.json", config)
    start = time.monotonic()
    manifest = dict(
        experiment="process_api",
        schema_version=1,
        batch_sizes=sizes,
        cpus=cpus,
        python=config["python"],
        binary_sha256=bench.file_hash(binary),
        harness_files=hashes,
        status="running",
        elapsed_seconds=0,
        load_average=os.getloadavg(),
    )
    manifest.update(bench.machine_metadata())
    bench.write_json(output / "manifest.json", manifest)
    try:
        result = subprocess.run(
            bench.pinned_command(
                [cpus[0]],
                [str(binary), "--json", "benchmark", "protocol", str(output / "config.json")],
            ),
            cwd=bench.ROOT,
            capture_output=True,
            text=True,
            timeout=args.budget or 300,
        )
        (output / "protocol.log").write_text(result.stderr)
        if result.returncode:
            raise RuntimeError("protocol helper failed; inspect protocol.log")
        bench.write_json(output / "measurements.json", json.loads(result.stdout))
        manifest["status"] = "completed"
    except BaseException as error:
        manifest.update(status="incomplete", error=str(error) or type(error).__name__)
        raise
    finally:
        manifest["elapsed_seconds"] = time.monotonic() - start
        bench.write_json(output / "manifest.json", manifest)
    return output


def summarize(measurement):
    rows = []
    for raw in measurement["rows"]:
        wall, callback = raw["wall_seconds"], raw["callback_seconds"]
        if (
            not wall
            or len(wall) != len(callback)
            or any(
                not math.isfinite(w) or not math.isfinite(c) or c < 0 or w < c
                for w, c in zip(wall, callback)
            )
        ):
            raise ValueError("invalid paired process timings")
        overhead = [w - c for w, c in zip(wall, callback)]
        rows.append(
            dict(
                operation=raw["operation"],
                batch=raw["batch"],
                feedback=raw["feedback"],
                calls=len(wall),
                wall_us=statistics.mean(wall) * 1e6,
                callback_us=statistics.mean(callback) * 1e6,
                overhead_us=statistics.mean(overhead) * 1e6,
                overhead_median_us=statistics.median(overhead) * 1e6,
                overhead_fraction=sum(overhead) / sum(wall),
            )
        )
    return rows


def report(directory):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.ticker import LogLocator, FuncFormatter

    path = directory / "measurements.json"
    measurement = json.loads(path.read_text()) if path.exists() else dict(rows=[])
    rows = summarize(measurement)
    bench.write_json(directory / "summary.json", rows)
    fig, axes = plt.subplots(2, 2, figsize=(12, 7), sharex=True, sharey="row")
    for column, operation in enumerate(["generate", "eval"]):
        for feedback, color in [(False, "#2563eb"), (True, "#c2410c")]:
            group = sorted(
                (r for r in rows if r["operation"] == operation and r["feedback"] == feedback),
                key=lambda r: r["batch"],
            )
            lookup = {r["batch"]: r["overhead_us"] for r in rows if r["operation"] == "feedback"}
            overhead = [
                r["overhead_us"]
                + (lookup.get(r["batch"], 0) if operation == "generate" and feedback else 0)
                for r in group
            ]
            sizes = [r["batch"] for r in group]
            for ax, ys in [
                (axes[0, column], [v / 1000 for v in overhead]),
                (axes[1, column], [v / n for v, n in zip(overhead, sizes)]),
            ]:
                ax.plot(
                    sizes,
                    ys,
                    "o-",
                    color=color,
                    label="With feedback" if feedback else "Without feedback",
                )
                ax.set_xscale("log", base=2)
                ax.set_yscale("log")
                ax.grid(alpha=0.2)
            axes[0, column].legend(fontsize=9)
        axes[0, column].set_title(
            "Sampler: flat generation + optional feedback"
            if operation == "generate"
            else "Evaluator"
        )
        batch_axis(axes[1, column], sorted({r["batch"] for r in rows}))
        axes[1, column].yaxis.set_major_locator(LogLocator(base=10, subs=(1, 2, 5)))
        axes[1, column].yaxis.set_major_formatter(FuncFormatter(lambda value, _: f"{value:g}"))
    axes[0, 0].set_ylabel("Adapter overhead per cycle (ms)")
    axes[1, 0].set_ylabel("Adapter overhead per sample (µs)")
    fig.suptitle("Process API · paired adapter wall time minus callback work")
    fig.tight_layout()
    figure_save(fig, directory, "overhead")
    plt.close(fig)
    notes = [
        "Real Rust adapters and Python SDK; no database. Three warmup calls are discarded; 32–128 measured calls per case in the current harness (saved raw timings retain the actual counts). Startup is excluded and retained separately.",
        "Overhead includes packing, validation, pipes, native generation decoding/evaluator accumulation, result cleanup and scheduling. It is not pure wire latency. Sampler training cycles sum generation and feedback costs at the same batch size.",
        "Sampler generation remains flat. Generation::into_batch() in older measurements only unwrapped a LatentBatchSpec; it did not expand evaluator Points. Both roles time result cleanup, with callback time subtracted per call.",
        "The generic evaluator returns per-sample values over IPC in both modes for accumulation; enabling feedback additionally retains weighted values. Similar evaluator curves are expected.",
    ]
    return publish(directory, "Process API overhead", rows, ["overhead.png"], notes)
