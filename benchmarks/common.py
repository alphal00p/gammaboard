"""Shared benchmark deployment, run lifecycle, cards and measurement contracts."""

import json
import math
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import hashlib
import platform

ROOT = Path(__file__).resolve().parents[1]


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n")


def file_hash(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def machine_metadata():
    return dict(
        platform=platform.platform(),
        python=sys.version.split()[0],
        cpu_model=next(
            (
                line.split(":", 1)[1].strip()
                for line in Path("/proc/cpuinfo").read_text().splitlines()
                if line.startswith("model name")
            ),
            "unknown",
        ),
        started_at=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    )


def physical_cpus():
    """One logical CPU per physical core, restricted to our existing affinity."""
    allowed = sorted(os.sched_getaffinity(0))
    seen, result = set(), []
    for cpu in allowed:
        base = Path(f"/sys/devices/system/cpu/cpu{cpu}/topology")
        key = (
            base.joinpath("physical_package_id").read_text().strip(),
            base.joinpath("core_id").read_text().strip(),
        )
        if key not in seen:
            seen.add(key)
            result.append(cpu)
    return result


def core_loads(candidates, seconds=0.25):
    def counters():
        return {
            int(label[3:]): (sum(v[:8]), v[3] + v[4])
            for line in Path("/proc/stat").read_text().splitlines()
            for label, *raw in [line.split()]
            if label.startswith("cpu") and label[3:].isdigit()
            for v in [list(map(int, raw))]
        }

    first = counters()
    time.sleep(seconds)
    last = counters()
    loads = {}
    for cpu in candidates:
        siblings = []
        for part in (
            Path(f"/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list")
            .read_text()
            .strip()
            .split(",")
        ):
            low, _, high = part.partition("-")
            siblings.extend(range(int(low), int(high or low) + 1))
        loads[cpu] = max(
            1 - (last[c][1] - first[c][1]) / max(1, last[c][0] - first[c][0]) for c in siblings
        )
    return loads


def idle_cpus(candidates, count):
    loads = core_loads(candidates)
    return sorted(sorted(candidates, key=lambda c: (loads[c], c))[:count])


class Session:
    def __init__(
        self,
        binary,
        output,
        budget,
        offset,
        max_connections=128,
        infrastructure_cpus=None,
        shared_buffers=None,
        database_directory="/tmp",
    ):
        self.binary, self.output, self.offset = binary, output, offset
        self.deadline = time.monotonic() + budget - 90  # Reserve graceful worker/database shutdown.
        self.directory = Path(tempfile.mkdtemp(prefix="gmb-scale-", dir=database_directory))
        self.runtime = self.directory / "runtime.toml"
        self.runtime.write_text(
            f'[resources]\nroots = [{json.dumps(str(self.directory/"resources"))}]\n'
            f'[local_postgres]\nsocket_dir = {json.dumps(str(self.directory/"socket"))}\nmax_connections = {max_connections}\n'
        )
        if shared_buffers is not None:
            with self.runtime.open("a") as stream:
                stream.write(f"shared_buffers = {json.dumps(shared_buffers)}\n")
        self.infrastructure_cpus = infrastructure_cpus
        self.workers = []

    def argv(self, *args):
        return [
            str(self.binary),
            "--runtime-config",
            str(self.runtime),
            "--port-offset",
            str(self.offset),
            "--json",
            *map(str, args),
        ]

    def cli(self, *args, timeout=90, cpus=None):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("suite time budget exhausted; partial results were retained")
        result = subprocess.run(
            pinned_command(cpus or self.infrastructure_cpus, self.argv(*args)),
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=min(timeout, remaining),
        )
        if result.returncode:
            raise RuntimeError(f"GammaBoard {args}: {result.stdout}\n{result.stderr}")
        return json.loads(result.stdout)

    def db_command(self, command):
        result = subprocess.run(
            pinned_command(self.infrastructure_cpus, self.argv("db", command)),
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=90,
        )
        with (self.output / "database.log").open("a") as log:
            log.write(result.stdout + result.stderr)
        if result.returncode:
            raise RuntimeError(f"database {command} failed; inspect database.log")

    def __enter__(self):
        try:
            with socket.socket() as sock:
                if sock.connect_ex(("127.0.0.1", 5400 + self.offset)) == 0:
                    raise RuntimeError("database port occupied; choose another --port-offset")
            self.db_command("start")
            self.cli("node", "list")
            return self
        except BaseException:
            self.__exit__(*sys.exc_info())
            raise

    def __exit__(self, exc_type, *_):
        errors = []
        started = time.monotonic()
        # Cleanup has its own allowance even when the measurement budget expires.
        self.deadline = time.monotonic() + 90
        if self.workers:
            try:
                self.cli("node", "stop", "--all")
                deadline = time.monotonic() + 75
                for worker in self.workers:
                    worker.wait(timeout=max(0.01, deadline - time.monotonic()))
                if any(worker.returncode != 0 for worker in self.workers):
                    errors.append("a worker exited unsuccessfully")
            except (RuntimeError, subprocess.TimeoutExpired) as error:
                errors.append(str(error))
        # Signal the whole owned fleet first, then use one deadline per wave.
        # A failed 512-worker shutdown must not wait seconds for each worker.
        for sig in (signal.SIGTERM, signal.SIGKILL):
            live = [worker for worker in self.workers if worker.poll() is None]
            for worker in live:
                try:
                    os.killpg(worker.pid, sig)
                except ProcessLookupError:
                    pass
            deadline = time.monotonic() + 5
            for worker in live:
                try:
                    worker.wait(timeout=max(0.01, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    pass
        if any(worker.poll() is None for worker in self.workers):
            errors.append("owned workers remained after forced cleanup")
        try:
            self.db_command("stop")
        except (RuntimeError, subprocess.SubprocessError, OSError) as error:
            errors.append(str(error))
        write_json(
            self.output / "cleanup.json",
            dict(
                clean=not errors,
                errors=errors,
                elapsed_seconds=time.monotonic() - started,
                runtime_directory=str(self.directory),
            ),
        )
        if exc_type is None and not errors:
            shutil.rmtree(self.directory)
        else:
            print(f"Deployment diagnostics retained in {self.directory}", file=sys.stderr)
        if errors and exc_type is None:
            raise RuntimeError("; ".join(errors))

    def start_pinned_workers(self, evaluator_cpus, sampler_cpu):
        """Pin each evaluator to its supplied CPU; sampler/database cores are separate."""
        nodes = [("bench-s", sampler_cpu, "bench_sampler")]
        nodes += [
            (f"bench-e-{index:03d}", cpu, "bench_evaluator")
            for index, cpu in enumerate(evaluator_cpus)
        ]
        for name, cpu, capability in nodes:
            with (self.output / f"{name}.log").open("w") as log:
                self.workers.append(
                    subprocess.Popen(
                        pinned_command(
                            cpu if isinstance(cpu, (list, tuple)) else [cpu],
                            self.argv(
                                "node", "run", "--name", name, "--capability", f"{capability}=1"
                            ),
                        ),
                        cwd=ROOT,
                        stdin=subprocess.DEVNULL,
                        stdout=log,
                        stderr=subprocess.STDOUT,
                        start_new_session=True,
                    )
                )
        deadline = min(self.deadline, time.monotonic() + 60)
        while time.monotonic() < deadline:
            if any(worker.poll() is not None for worker in self.workers):
                raise RuntimeError("a pinned worker exited; inspect its log")
            if len(self.cli("node", "list")) == len(nodes):
                return
            time.sleep(0.25)
        raise TimeoutError("pinned worker registration timed out")


def pinned_command(cpus, command):
    return ["taskset", "-c", ",".join(map(str, cpus)), *command] if cpus else command


def run_card(
    batch_size, min_tick_time_ms=10, telemetry_interval_ms=250, generation_batch_size=None
):
    return f"""name = "scaling-benchmark"
[evaluator]
kind = "unit"
continuous_dims = 6
cpu_iterations_per_sample = 0
[evaluator_runner_params]
min_tick_time_ms = {min_tick_time_ms}
performance_snapshot_interval_ms = {telemetry_interval_ms}
[sampler_aggregator_runner_params]
min_tick_time_ms = {min_tick_time_ms}
frontend_sync_interval_ms = {telemetry_interval_ms}
performance_snapshot_interval_ms = {telemetry_interval_ms}
[sampler_aggregator_runner_params.queue]
fixed_batch_size = {batch_size}
max_batch_size = {batch_size}
[[task_queue]]
name = "measure"
kind = "sample"
stop_condition = {{ max_samples = 1000000000000 }}
accumulator = {{ config = "scalar" }}
sampler_aggregator = {{ config = {{ kind = "naive_monte_carlo", seed = 1234, generation_batch_size = {generation_batch_size or 1048576} }} }}
"""


def load_results(directory):
    path = directory / "results.jsonl"
    return (
        [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
        if path.exists()
        else []
    )


def preserve_inputs(output, binary):
    """A later build or source edit must not change an experiment already running."""
    # An all-suite run shares one binary/source snapshot across its families.
    parent = output.parent
    shared = (parent / "manifest.json").exists() and json.loads(
        (parent / "manifest.json").read_text()
    ).get("experiment") == "suite"
    inputs = (parent if shared else output) / "inputs"
    copied = inputs / "gammaboard"
    if not inputs.exists():
        inputs.mkdir()
        shutil.copy2(binary, copied)
        shutil.copytree(
            ROOT / "benchmarks", inputs / "benchmarks", ignore=shutil.ignore_patterns("__pycache__")
        )
        migrations = Path(os.environ.get("GAMMABOARD_MIGRATIONS_DIR", ROOT / "migrations"))
        shutil.copytree(migrations, inputs / "migrations")
    os.environ["GAMMABOARD_MIGRATIONS_DIR"] = str(inputs / "migrations")
    return copied, {
        str(p.relative_to(inputs)): file_hash(p)
        for p in (inputs / "benchmarks").rglob("*")
        if p.is_file()
    }


def activity(measurement):
    first, last = measurement["snapshots"][0], measurement["snapshots"][-1]
    result = {}
    for collection, field, role in [
        ("evaluators", "metrics", "evaluator"),
        ("samplers", "runtime_metrics", "sampler"),
    ]:
        start = {r["worker_id"]: r[field] for r in first[collection]}
        end_ids = [r["worker_id"] for r in last[collection]]
        if (
            not start
            or set(start) != set(end_ids)
            or len(start) != len(first[collection])
            or len(set(end_ids)) != len(end_ids)
        ):
            raise ValueError("missing, duplicate or changed worker coverage")
        total = dict(elapsed_seconds=0.0, compute_seconds=0.0, io_seconds=0.0)
        for row in last[collection]:
            a, b = start[row["worker_id"]], row[field]
            epoch = "epoch" if role == "evaluator" else "runner_epoch"
            if any(
                a.get(k) is None or a.get(k) != b.get(k) for k in [epoch, "node_uuid", "task_id"]
            ):
                raise ValueError("worker identity changed")
            delta = {k: b["busy"][k] - a["busy"][k] for k in total}
            elapsed = delta["elapsed_seconds"]
            if elapsed <= 0 or any(
                not math.isfinite(v) or v < 0 or v > elapsed + 1e-9 for v in delta.values()
            ):
                raise ValueError("invalid busy interval")
            for k, v in delta.items():
                total[k] += v
        for lane in ["compute", "io"]:
            result[f"{role}_{lane}"] = 100 * total[f"{lane}_seconds"] / total["elapsed_seconds"]
    return result
