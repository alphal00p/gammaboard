#!/usr/bin/env python3
"""GammaBoard benchmarks: one command produces measurements, plots and a local report index."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import tomllib

from .common import ROOT, physical_cpus, write_json

FAMILIES = ("frontier", "sampler-io", "evaluator-io", "protocol")


def parser():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("command", choices=("all", *FAMILIES, "report", "plan"))
    cli.add_argument("--output", type=Path, default=None)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    cli.add_argument(
        "--binary",
        type=Path,
        default=Path(os.environ.get("GAMMABOARD_BINARY", target / "dev-optim/gammaboard")),
    )
    cli.add_argument("--python", type=Path, default=Path(sys.executable))
    cli.add_argument(
        "--quick",
        action="store_true",
        help="fewer frontier counts, one I/O repeat, shorter measurement windows",
    )
    cli.add_argument("--suite", type=Path, default=ROOT / "benchmarks/frontier.toml")
    cli.add_argument(
        "--max-evaluators", type=int, help="frontier count cap (default 512; quick 64)"
    )
    cli.add_argument("--workers", type=int, nargs="+", help="explicit frontier evaluator counts")
    cli.add_argument("--points", type=Path, help="explicit frontier points in JSON")
    cli.add_argument(
        "--include-rng", action="store_true", help="also run compact RNG frontier curves"
    )
    cli.add_argument(
        "--batch-sizes", type=int, nargs="+", help="override I/O and protocol batch sizes"
    )
    cli.add_argument(
        "--io-threads",
        type=int,
        nargs="+",
        help="sampler I/O sweep (default 1 2 4 8, limited by CPU budget); frontier uses first value if provided",
    )
    cli.add_argument(
        "--consumers", type=int, default=16, help="lightweight consumers for sampler I/O"
    )
    cli.add_argument(
        "--duration", type=float, help="I/O measured seconds per trial (default 4; quick 2)"
    )
    cli.add_argument("--warmup", type=float, help="I/O warmup seconds (default 1; quick 0.5)")
    cli.add_argument("--repetitions", type=int, help="I/O repetitions (default 2; quick 1)")
    cli.add_argument(
        "--memory-mib",
        type=int,
        default=2048,
        help="I/O payload working budget; database cache is additional",
    )
    cli.add_argument(
        "--cpu-limit", type=int, help="physical core budget; roles use disjoint CPU sets"
    )
    cli.add_argument(
        "--database-cores",
        type=int,
        help="I/O database CPU allocation (default balanced within --cpu-limit)",
    )
    cli.add_argument(
        "--database-cache-mib",
        type=int,
        default=2048,
        help="I/O PostgreSQL shared buffers, independent of payload memory",
    )
    cli.add_argument(
        "--insert-concurrency",
        type=int,
        default=4,
        help="sampler I/O inserts in flight; pool has two extra connections",
    )
    cli.add_argument(
        "--queue-batches",
        type=int,
        default=64,
        help="sampler I/O outstanding batches, subject to payload memory budget",
    )
    cli.add_argument(
        "--profile", action="store_true", help="sample database wait states during I/O measurements"
    )
    cli.add_argument(
        "--budget", type=int, help="time limit per family in seconds, including cleanup"
    )
    cli.add_argument(
        "--database-directory",
        type=Path,
        default=Path("/tmp"),
        help="parent directory for private databases; choose a filesystem to compare storage",
    )
    cli.add_argument("--port-offset", type=int, default=150)
    return cli


def frontier_suite(args):
    from . import frontier

    suite = tomllib.loads(args.suite.read_text())
    cpus = physical_cpus()
    count = min(len(cpus), args.cpu_limit or len(cpus))
    if count < 3:
        raise ValueError("frontier requires at least three physical cores")
    maximum = (
        args.max_evaluators if args.max_evaluators is not None else (64 if args.quick else 512)
    )
    if not 1 <= maximum <= 512:
        raise ValueError("--max-evaluators must be 1..512")
    suite["workers"] = args.workers or ([1, 4, 16, maximum] if args.quick else suite["workers"])
    if args.workers and (
        len(args.workers) != len(set(args.workers))
        or any(n < 1 or n > maximum for n in args.workers)
    ):
        raise ValueError("explicit worker counts must be unique and within --max-evaluators")
    suite["workers"] = sorted({n for n in suite["workers"] if n <= maximum})
    suite["modes"] = ["materialized", "training"] + (["rng"] if args.include_rng else [])
    suite["infrastructure_cores"] = min(suite["infrastructure_cores"], count - 1)
    suite["sampler_io_threads"] = (
        args.io_threads[0] if args.io_threads else suite.get("sampler_io_threads", 1)
    )
    if args.io_threads is None:
        suite["sampler_io_threads"] = min(
            suite["sampler_io_threads"], suite["infrastructure_cores"] - 1
        )
    if suite["sampler_io_threads"] >= suite["infrastructure_cores"]:
        raise ValueError(
            "CPU/infrastructure budget must cover sampler I/O threads and a database core"
        )
    if args.quick:
        suite["measurement_seconds"] = 6
    if args.budget:
        suite["budget_seconds"] = args.budget
    # Preserve the chosen batches on large hosts; constrain estimated residency
    # explicitly on smaller machines. Save the effective plan with every result.
    available = (
        int(
            next(
                line.split()[1]
                for line in Path("/proc/meminfo").read_text().splitlines()
                if line.startswith("MemAvailable:")
            )
        )
        * 1024
    )
    cgroup = Path("/sys/fs/cgroup")
    if (cgroup / "memory.max").exists():
        limit = (cgroup / "memory.max").read_text().strip()
        if limit != "max":
            available = min(
                available, max(0, int(limit) - int((cgroup / "memory.current").read_text()))
            )
    suite["sample_memory_budget"] = min(suite["sample_memory_budget"], int(available * 0.4 / 160))
    if available < 16 * 1024**3:
        suite["database_shared_buffers"] = "256MB"
    return frontier.validate(suite)


def report(directory):
    kind = json.loads((directory / "manifest.json").read_text())["experiment"]
    if kind == "frontier":
        from . import frontier_plots as module
    elif kind == "io":
        from . import io as module
    elif kind == "process_api":
        from . import protocol as module
    else:
        from .reporting import aggregate

        return aggregate(directory, [directory / name for name in FAMILIES])
    return module.report(directory)


def run_family(args):
    if args.command == "frontier":
        from . import frontier as module

        args.effective_suite = frontier_suite(args)
    elif args.command == "protocol":
        from . import protocol as module
    else:
        from . import io as module
    # Each family may restrict its affinity; restore it before the next one.
    affinity = os.sched_getaffinity(0)
    environment = dict(os.environ)
    failure = None
    existed = args.output.exists()
    try:
        module.execute(args)
    except BaseException as error:
        failure = error
        raise
    finally:
        os.sched_setaffinity(0, affinity)
        os.environ.clear()
        os.environ.update(environment)
        if not existed and (args.output / "manifest.json").exists():
            try:
                print(f"Report: {report(args.output)}", flush=True)
            except Exception as error:
                if failure is None:
                    raise
                print(f"Report generation failed: {error}", file=sys.stderr)


def main(argv=None):
    args = parser().parse_args(argv)
    args.output = (
        args.output or ROOT / "results" / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    ).absolute()
    args.duration = args.duration if args.duration is not None else (2 if args.quick else 4)
    args.warmup = args.warmup if args.warmup is not None else (0.5 if args.quick else 1)
    args.repetitions = (
        args.repetitions if args.repetitions is not None else (1 if args.quick else 2)
    )
    try:
        if args.budget is not None and args.budget < 120:
            raise ValueError("budget must allow at least 120s including setup/cleanup")
        if args.command == "report":
            print(report(args.output))
            return 0
        if args.command == "plan":
            from . import frontier

            suite = frontier_suite(args)
            points = frontier.sparse_points(suite)
            seconds = sum(frontier.measurement_seconds(suite, p) for p in points)
            print(json.dumps(suite, indent=2))
            print(
                f"{len(points)} frontier points; {seconds/60:.1f} measured minutes plus setup, warmup and cleanup."
            )
            return 0
        if not 1 <= args.port_offset <= 57000:
            raise ValueError("invalid port offset")
        if args.cpu_limit is not None and args.cpu_limit < 3:
            raise ValueError("CPU limit must be at least three")
        args.binary = args.binary.resolve(strict=True)
        # Fail before deploying if reporting or process dependencies are unavailable.
        os.environ.setdefault("MPLCONFIGDIR", str(args.output.parent / ".matplotlib"))
        import matplotlib

        if args.command in ("all", "protocol"):
            subprocess.run(
                [str(args.python), "-c", "import numpy"], check=True, capture_output=True
            )
        for executable in (
            ["taskset"] if args.command == "protocol" else ["initdb", "pg_ctl", "psql", "taskset"]
        ):
            if not shutil.which(executable):
                raise ValueError(f"missing {executable}; see docs/benchmarking.md")
        if args.command == "all":
            from .reporting import aggregate

            args.output.mkdir(parents=True, exist_ok=False)
            root = args.output
            started = time.monotonic()
            children = []
            manifest = dict(experiment="suite", status="running", elapsed_seconds=0)
            write_json(root / "manifest.json", manifest)
            try:
                for name in FAMILIES:
                    child = argparse.Namespace(**vars(args))
                    child.command = name
                    child.output = root / name
                    children.append(child.output)
                    run_family(child)
                statuses = {
                    child.name: json.loads((child / "manifest.json").read_text())["status"]
                    for child in children
                }
                manifest["families"] = statuses
                manifest["status"] = (
                    "completed"
                    if all(s == "completed" for s in statuses.values())
                    else "completed_with_flags"
                )
            except BaseException:
                manifest["status"] = "incomplete"
                raise
            finally:
                manifest["elapsed_seconds"] = time.monotonic() - started
                write_json(root / "manifest.json", manifest)
                print(f"Suite report: {aggregate(root,children)}")
        else:
            run_family(args)
        manifest = json.loads((args.output / "manifest.json").read_text())
        states = list(manifest.get("families", {}).values()) or [manifest["status"]]
        if any(s not in ("completed", "completed_with_flags") for s in states):
            return 1
    except (
        ValueError,
        RuntimeError,
        TimeoutError,
        OSError,
        subprocess.SubprocessError,
        KeyboardInterrupt,
    ) as error:
        print(f"Benchmark stopped: {str(error) or type(error).__name__}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
