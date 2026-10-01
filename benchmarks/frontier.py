"""Sparse deployment scaling with fresh runs and one measurement contract."""

from dataclasses import asdict, dataclass
from datetime import datetime
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import time
import tomllib
import traceback

from . import common as bench

MODES = ("rng", "materialized", "training")
TIMING_JITTER = dict(relative_sigma=0.1, seed=1234)
SPARSE_BATCHES = {5: 65536, 200: 4096, 5000: 256}
VALUE_COORDINATE = 0
# Large fleets must drain more than 100 million samples even at zero delay.
# This bounds warmup only; measurement duration and acceptance stay unchanged.
WARMUP_TIMEOUT_SECONDS = 300


@dataclass(frozen=True)
class Point:
    mode: str
    eval_us: float
    workers: int
    batch: int


def validate(suite):
    expected = {
        "eval_us",
        "workers",
        "modes",
        "budget_seconds",
        "max_batch_size",
        "sample_memory_budget",
        "infrastructure_cores",
        "measurement_seconds",
        "max_batch_seconds",
        "generation_batch_size",
        "min_tick_time_ms",
        "telemetry_interval_ms",
        "insert_concurrency",
    }
    allowed = expected | {"sampler_db_pool_size", "sampler_io_threads", "database_shared_buffers"}
    if expected - suite.keys() or suite.keys() - allowed:
        raise ValueError(
            f"suite keys missing={expected-suite.keys()}, unknown={suite.keys()-allowed}"
        )
    if (
        type(suite.get("sampler_db_pool_size", 6)) is not int
        or not 1 <= suite.get("sampler_db_pool_size", 6) <= 6
    ):
        raise ValueError("sampler_db_pool_size must be between 1 and 6")
    if (
        type(suite.get("sampler_io_threads", 1)) is not int
        or suite.get("sampler_io_threads", 1) < 1
    ):
        raise ValueError("sampler_io_threads must be a positive integer")
    buffers = suite.get("database_shared_buffers", "256MB")
    if not isinstance(buffers, str) or not re.fullmatch(r"[1-9][0-9]*(MB|GB)", buffers):
        raise ValueError("database_shared_buffers must be a positive size in MB or GB")
    for name, low, high, integral in [("eval_us", 0, 100000, False), ("workers", 1, 512, True)]:
        values = suite[name]
        if (
            not values
            or len(set(values)) != len(values)
            or any(
                type(v) not in ((int,) if integral else (int, float))
                or not math.isfinite(v)
                or not low <= v <= high
                for v in values
            )
        ):
            raise ValueError(f"invalid {name}")
    if (
        not suite["modes"]
        or len(set(suite["modes"])) != len(suite["modes"])
        or set(suite["modes"]) - set(MODES)
    ):
        raise ValueError("invalid modes")
    for name, low, high in [
        ("budget_seconds", 120, 7200),
        ("max_batch_size", 16, 16777216),
        ("generation_batch_size", 16, 16777216),
        ("sample_memory_budget", 1024, 2147483648),
        ("infrastructure_cores", 2, 16),
        ("measurement_seconds", 6, 120),
        ("max_batch_seconds", 0.1, 30),
        ("min_tick_time_ms", 0, 100),
        ("telemetry_interval_ms", 100, 2000),
        ("insert_concurrency", 1, 8),
    ]:
        value = suite[name]
        integral = name in {
            "max_batch_size",
            "generation_batch_size",
            "sample_memory_budget",
            "infrastructure_cores",
            "min_tick_time_ms",
            "telemetry_interval_ms",
            "insert_concurrency",
        }
        if (
            type(value) not in ((int,) if integral else (float, int))
            or not math.isfinite(value)
            or not low <= value <= high
        ):
            raise ValueError(f"invalid {name}")
    return suite


def worker_cpus(candidates, loads, workers, infrastructure_cores):
    ordered = sorted(candidates, key=lambda c: (loads[c], c))
    infrastructure = ordered[:infrastructure_cores]
    available = ordered[infrastructure_cores : infrastructure_cores + max(workers)]
    if not available:
        raise ValueError("need an evaluator core in addition to infrastructure cores")
    # Delayed workers need not reserve an otherwise idle physical core each.
    return infrastructure, [available[i % len(available)] for i in range(max(workers))]


def floor_power_two(value):
    return 2 ** max(4, math.floor(math.log2(max(16, value))))


def batch_limit(suite, cost, workers):
    # Budget covers evaluator batches plus queued/buffered materialized work.
    # It is a sample-count bound, not a claim about exact allocator overhead.
    return floor_power_two(
        min(
            suite["max_batch_size"],
            suite["max_batch_seconds"] / cost if cost else math.inf,
            suite["sample_memory_budget"] / (4 * workers + 2),
        )
    )


def sparse_points(suite):
    """Replay selected settings, with sparse delayed curves and a full zero curve.

    Extra 5-us anchors cover the knees observed in the earlier tuning study.
    These are reproducible measurement settings, not a new optimization search.
    """
    workers = sorted(suite["workers"])
    points = []
    costs = sorted(suite["eval_us"])
    order = []
    while costs:
        order.append(costs.pop(0))
        if costs:
            order.append(costs.pop())
    for cost in order:
        if cost != 0 and cost not in SPARSE_BATCHES:
            raise ValueError(
                "sparse curves support delays 0, 5, 200 and 5000 µs; use --points for other delays"
            )
        for mode in suite["modes"]:
            counts = set(workers if not cost else workers[::2] + [workers[-1]])
            if cost == 5:
                counts |= {128 if mode == "rng" else 8} & set(workers)
            batch = (524288 if mode == "rng" else 65536) if not cost else SPARSE_BATCHES[cost]
            points.extend(
                Point(mode, cost, n, min(batch, batch_limit(suite, cost / 1e6, n)))
                for n in sorted(counts)
            )
    return points


def validate_points(points, suite):
    if not points:
        raise ValueError("validation points must not be empty")
    for point in points:
        if (
            point.mode not in suite["modes"]
            or point.eval_us not in suite["eval_us"]
            or type(point.workers) is not int
            or point.workers not in suite["workers"]
            or type(point.batch) is not int
            or not 16 <= point.batch <= batch_limit(suite, point.eval_us / 1e6, point.workers)
        ):
            raise ValueError(f"invalid validation point: {point}")
    return points


def generation_batch_size(suite, eval_us):
    return floor_power_two(
        min(
            suite["generation_batch_size"],
            suite["sample_memory_budget"] / 2,
            suite["max_batch_seconds"] * 1e6 / eval_us if eval_us else math.inf,
        )
    )


def measurement_seconds(suite, point):
    batch_seconds = point.batch * point.eval_us / 1e6
    # Insert bundles can complete out of ID order in every mode. Collection
    # advances in bursts; cover generation cycles as well as evaluator batches.
    generation_seconds = (
        generation_batch_size(suite, point.eval_us) * point.eval_us / 1e6 / point.workers
    )
    return max(
        suite["measurement_seconds"],
        6 * batch_seconds,
        10 * batch_seconds / point.workers,
        2 * generation_seconds,
    )


def progress_agrees(point, accepted, evaluated):
    # Worker telemetry can trail by a completion at either endpoint. Larger
    # discrepancies indicate a burst/backlog window, not sustained throughput.
    return abs(accepted - evaluated) <= max(2 * point.batch, 0.1 * max(accepted, evaluated))


def queue_settings(point, suite):
    return dict(
        fixed_batch_size=point.batch,
        max_batch_size=point.batch,
        max_batches_per_tick=128,
        completed_batch_fetch_limit=max(100, 2 * max(suite["workers"])),
        max_insert_bundle_size=5,
        max_concurrent_insert_tasks=suite["insert_concurrency"],
    )


def card(mode, eval_us, suite, value_coordinate=VALUE_COORDINATE):
    generation_size = generation_batch_size(suite, eval_us)
    text = bench.run_card(
        16, suite["min_tick_time_ms"], suite["telemetry_interval_ms"], generation_size
    )
    text = text.replace(
        "[sampler_aggregator_runner_params]\n",
        f'[sampler_aggregator_runner_params]\ndb_pool_size = {suite.get("sampler_db_pool_size", 6)}\n'
        f'io_threads = {suite.get("sampler_io_threads", 1)}\n',
    )
    if value_coordinate is not None:
        text = text.replace(
            "cpu_iterations_per_sample = 0",
            f"value_coordinate = {value_coordinate}\ncpu_iterations_per_sample = 0",
        )
    text = text.replace(
        "cpu_iterations_per_sample = 0",
        f"cpu_iterations_per_sample = 0\ntiming = {{ per_sample_seconds = {eval_us/1e6}, "
        f'sigma_per_sample_seconds = {eval_us/1e6*TIMING_JITTER["relative_sigma"]}, '
        f'seed = {TIMING_JITTER["seed"]} }}',
    )
    text = text.replace(
        'name = "scaling-benchmark"',
        'name = "live-frontier"\n'
        "evaluator_requirements = { bench_evaluator = 1 }\n"
        "sampler_requirements = { bench_sampler = 1 }",
    )
    text = text.replace("max_samples = 1000000000000", "max_samples = 1000000000000000")
    if mode == "training":
        # A fixed large window exposes transport/ingestion, without multiplying
        # the experiment by optimizer latency or hiding a window scaled with N.
        text = text.replace(
            'kind = "naive_monte_carlo", seed = 1234',
            'kind = "naive_monte_carlo", seed = 1234, training_window_samples = 1000000000000',
        )
    if mode == "rng":
        header = text.split("[[task_queue]]")[0]
        text = header + """[[task_queue]]
name = "initialize-grid"
kind = "sample"
stop_condition = { max_samples = 16 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "havana_training", seed = 1234, bins = 2, samples_for_update = 16, initial_training_rate = 0.0, final_training_rate = 0.0 } }
[[task_queue]]
name = "measure"
kind = "sample"
stop_condition = { max_samples = 1000000000000000 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "havana_inference", seed = 1234, generation_batch_size = GENERATION_SIZE } }
""".replace("GENERATION_SIZE", str(generation_size))
    return text


def toml_settings(settings):
    return "\n".join(f"{key} = {json.dumps(value)}" for key, value in settings.items()) + "\n"


def active_evaluators(snapshot):
    names = {
        n["name"]
        for n in snapshot["nodes"]
        if n["live"]
        and n["active_run_id"] == snapshot["run_id"]
        and n["active_role"] == "evaluator"
    }
    return [row for row in snapshot["evaluators"] if row["worker_id"] in names]


def runtime(snapshot):
    if len(snapshot["samplers"]) != 1:
        raise ValueError("one sampler is required")
    return snapshot["samplers"][0]["runtime_metrics"]


def applied(snapshot, settings):
    if len(snapshot["samplers"]) != 1:
        return False
    config = snapshot["samplers"][0]["engine_diagnostics"].get("runner", {}).get("queue_config", {})
    return all(config.get(k) == v for k, v in settings.items())


def warmed(snapshot, baseline, point, completed_offset=0):
    # Wait past all work generated before the tuning acknowledgement, including
    # an undispatched draw, then accept two new batches per worker. No pause knob.
    previous = runtime(baseline)
    buffered = baseline["samplers"][0]["engine_diagnostics"]["runner"]["buffered_generated_samples"]
    cutoff = max(
        previous["completed_samples_total"],
        previous["produced_samples_total"] + buffered + completed_offset,
    )
    return runtime(snapshot)["completed_samples_total"] >= cutoff + 2 * point.workers * point.batch


def sampler_progress(measurement):
    """Measure accepted samples and feedback on one sampler telemetry window.

    The top-level run counter is checkpointed separately and can lag differently
    at either endpoint; CLI wall time is not its publication interval either.
    """
    first, last = [runtime(s) for s in (measurement["snapshots"][0], measurement["snapshots"][-1])]
    if any(
        first.get(k) is None or first[k] != last.get(k)
        for k in ["runner_epoch", "node_uuid", "task_id"]
    ):
        raise RuntimeError("sampler identity changed during measurement")
    accepted = last["completed_samples_total"] - first["completed_samples_total"]
    ingested = last["ingested_samples_total"] - first["ingested_samples_total"]
    elapsed = last["busy"]["elapsed_seconds"] - first["busy"]["elapsed_seconds"]
    if accepted < 0 or ingested < 0 or not math.isfinite(elapsed) or elapsed <= 0:
        raise RuntimeError("invalid sampler progress window")
    return accepted, ingested, elapsed


def generation_window(measurement, point, suite, completed_offset=0):
    """Resolve a burst window using the first and last generation completions.

    Ordered collection can jump over several batches at once. Matching cycle
    boundaries removes partial-generation backlog from the rate, without picking
    an interval based on its throughput. Prefer the full observation when its
    deltas already agree, and always preserve that observation separately.
    """
    accepted, _, _ = sampler_progress(measurement)
    evaluated = sum(r["samples_evaluated"] for r in measurement["evaluator_deltas"])
    if not measurement["valid"] or progress_agrees(point, accepted, evaluated):
        return measurement
    snapshots = measurement["snapshots"]
    generation = generation_batch_size(suite, point.eval_us)
    boundaries = [
        i
        for i in range(1, len(snapshots))
        if (runtime(snapshots[i])["completed_samples_total"] - completed_offset) % generation == 0
        and runtime(snapshots[i])["completed_samples_total"]
        != runtime(snapshots[i - 1])["completed_samples_total"]
    ]
    if len(boundaries) < 2:
        return measurement
    selected = snapshots[boundaries[0] : boundaries[-1] + 1]
    accepted, _, elapsed = sampler_progress(dict(snapshots=selected))
    _, _, observed = sampler_progress(measurement)
    if accepted < 8 * point.batch or elapsed < observed / 2:
        return measurement
    first = {w["worker_id"]: w for w in active_evaluators(selected[0])}
    last = {w["worker_id"]: w for w in active_evaluators(selected[-1])}
    if first.keys() != last.keys():
        return measurement
    deltas = []
    for worker, finish in last.items():
        start = first[worker]
        a, b = start["metrics"], finish["metrics"]
        if any(a[k] != b[k] for k in ("epoch", "node_uuid", "task_id")):
            return measurement
        deltas.append(
            dict(
                worker_id=worker,
                epoch=b["epoch"],
                since=start["created_at"],
                until=finish["created_at"],
                **{k: b[k] - a[k] for k in ("samples_evaluated", "batches_completed")},
                **{k: b["cumulative"][k] - v for k, v in a["cumulative"].items()},
            )
        )
    return dict(
        valid=measurement["valid"],
        issues=list(measurement["issues"]),
        snapshots=selected,
        evaluator_deltas=deltas,
        elapsed_seconds=elapsed,
        window_selection=dict(
            kind="completed_generations",
            first_snapshot=boundaries[0],
            last_snapshot=boundaries[-1],
            generation_samples=generation,
            observation_seconds=observed,
        ),
    )


def adequate(row):
    # Evaluator completion alone does not establish accepted/ingested progress.
    accepted = row.get("accepted_samples")
    if accepted is None and row.get("rate") is not None and "elapsed_seconds" in row:
        accepted = round(row["rate"] * row["elapsed_seconds"])
    return bool(row.get("adequate") and (accepted is None or accepted >= 8 * row["point"]["batch"]))


def configuration_key(row):
    return tuple(sorted(row["point"].items())), tuple(sorted(row["settings"].items()))


def choose(rows):
    """Maximize accepted throughput, then prefer cheaper settings within 5%."""
    valid = [r for r in rows if r.get("valid", True) and adequate(r)]
    if not valid:
        return None
    peak = max(r["rate"] for r in valid)
    return min(
        (r for r in valid if r["rate"] >= 0.95 * peak),
        key=lambda r: (r["point"]["workers"], r["point"]["batch"], -r["rate"]),
    )


def summarize(records):
    groups = {}
    for row in records:
        if row.get("valid") and adequate(row):
            key = configuration_key(row)
            groups.setdefault(key, []).append(row)
    rows = []
    for group in groups.values():
        values = [r["rate"] for r in group]
        rows.append(
            dict(
                point=group[0]["point"],
                rate=statistics.median(values),
                adequate=True,
                trials=len(group),
                settings=group[-1]["settings"],
                observed_batch_size=statistics.median(r["observed_batch_size"] for r in group),
                batches_per_second=statistics.median(r["batches_per_second"] for r in group),
                rss_bytes=max(r["rss_bytes"] for r in group),
            )
        )
    envelope = []
    for mode, cost, workers in sorted(
        {(r["point"]["mode"], r["point"]["eval_us"], r["point"]["workers"]) for r in rows}
    ):
        group = [
            r
            for r in rows
            if (r["point"]["mode"], r["point"]["eval_us"], r["point"]["workers"])
            == (mode, cost, workers)
        ]
        envelope.append(choose(group))
    return dict(configurations=rows, frontier=envelope)


def measurement_row(measured, point, completed_offset=0):
    accepted, ingested, elapsed = sampler_progress(measured)
    deltas = measured["evaluator_deltas"]
    batches = sum(r["batches_completed"] for r in deltas)
    samples = sum(r["samples_evaluated"] for r in deltas)
    last = measured["snapshots"][-1]
    r = runtime(last)
    for snapshot in measured["snapshots"]:
        snapshot["evaluators"] = active_evaluators(snapshot)
    row = dict(
        valid=measured["valid"],
        issues=measured["issues"],
        rate=accepted / elapsed,
        adequate=batches >= 8
        and accepted >= 8 * point.batch
        and progress_agrees(point, accepted, samples),
        evaluated_samples=samples,
        progress_agrees=progress_agrees(point, accepted, samples),
        accepted_samples=accepted,
        ingested_samples_delta=ingested,
        completed_batches=batches,
        elapsed_seconds=elapsed,
        window_selection=measured.get("window_selection"),
        observed_batch_size=samples / batches if batches else 0,
        batches_per_second=batches / elapsed,
        rss_bytes=sum(
            w["rss_bytes"] or 0 for group in ["evaluators", "samplers"] for w in last[group]
        ),
        produced_samples=r["produced_samples_total"],
        completed_samples=r["completed_samples_total"],
        ingested_samples=r["ingested_samples_total"],
        backlog_samples=r["produced_samples_total"]
        + completed_offset
        - r["completed_samples_total"],
    )
    if not row["progress_agrees"]:
        row["issues"].append(
            "accepted and evaluated sample deltas disagree; unresolved backlog window"
        )
    row["ideal_samples_per_second"] = point.workers * 1e6 / point.eval_us if point.eval_us else None
    row["ideal_fraction"] = row["rate"] / row["ideal_samples_per_second"] if point.eval_us else None
    first = {
        w["worker_id"]: w["metrics"]["engine_diagnostics"].get("timing")
        for w in measured["snapshots"][0]["evaluators"]
    }
    timings = [
        (first.get(w["worker_id"]), w["metrics"]["engine_diagnostics"].get("timing"))
        for w in last["evaluators"]
    ]
    for name in ["requested_seconds", "actual_seconds"]:
        row[f"sleep_{name}"] = (
            sum(after[name] - before[name] for before, after in timings)
            if timings and all(before and after for before, after in timings)
            else None
        )
    try:
        row.update(bench.activity(measured))
    except ValueError as exc:
        row["activity_issue"] = str(exc)
    payload = r["queue"]["rolling"]["insert_bundle_payload_bytes_per_batch"]
    row["input_bytes_per_batch"] = payload.get("mean")
    if (
        point.mode == "rng"
        and payload.get("mean")
        and payload["mean"] > 128 + 40 * math.ceil(point.batch / 1024)
    ):
        row["valid"] = False
        row["issues"].append("RNG mode emitted materialized inputs")
    if point.mode == "training" and ingested <= 0:
        row["valid"] = False
        row["issues"].append("training feedback did not reach the sampler")
    return row


class LiveRun:
    def __init__(self, session, output, suite, mode, cost, initial_point=None):
        self.session, self.output, self.suite = session, output, suite
        self.mode, self.cost, self.delay = mode, cost, cost / 1e6
        self.initial_point = initial_point
        self.fresh_start = initial_point is not None
        self.run = None
        self.task = None
        self.workers = 0
        self.settings = None

    def snapshot(self):
        return self.session.cli("run", "performance", self.run)

    def wait(self, predicate, seconds=60):
        deadline = min(self.session.deadline, time.monotonic() + seconds)
        snapshot = None
        while time.monotonic() < deadline:
            snapshot = self.snapshot()
            if snapshot["failed_tasks"]:
                raise RuntimeError(f"run {self.run} has failed tasks")
            if predicate(snapshot):
                return snapshot
            time.sleep(0.2)
        bench.write_json(self.output / "unsettled.json", snapshot)
        raise TimeoutError(f"run {self.run} did not settle within {seconds}s")

    def __enter__(self):
        self.output.mkdir()
        path = self.output / "run.toml"
        text = card(self.mode, self.cost, self.suite)
        self.workers = self.initial_point.workers if self.initial_point else 1
        if self.initial_point:
            self.settings = queue_settings(self.initial_point, self.suite)
            header, rest = text.split("[sampler_aggregator_runner_params.queue]")
            _, tasks = rest.split("[[task_queue]]", 1)
            text = (
                header
                + "[sampler_aggregator_runner_params.queue]\n"
                + toml_settings(self.settings)
                + "\n[[task_queue]]"
                + tasks
            )
        path.write_text(text)
        self.run = self.session.cli("run", "create", path)["run_id"]
        try:
            # Initialize the tiny Havana grid before attaching the full fleet;
            # otherwise hundreds of workers immediately cross a task boundary.
            initial_workers = 1 if self.mode == "rng" else self.workers
            self.session.cli("run", "resume", self.run, "--max-evaluators", initial_workers)
            tasks = self.session.cli("run", "task", "list", self.run)
            self.task = next(str(t["id"]) for t in tasks if t["name"] == "measure")
            self.wait(
                lambda s: s["task_id"] == self.task
                and len(s["samplers"]) == 1
                and len(active_evaluators(s)) >= 1
            )
            if self.workers > initial_workers:
                self.session.cli(
                    "run", "resume", self.run, "--max-evaluators", self.workers - initial_workers
                )
            tasks = self.session.cli("run", "task", "list", self.run)
            self.completed_offset = sum(
                t["nr_completed_samples"] for t in tasks if str(t["id"]) != self.task
            )
            self.settings = self.settings or queue_settings(
                Point(self.mode, self.cost, 1, 16), self.suite
            )
            return self
        except BaseException:
            self.__exit__(*__import__("sys").exc_info())
            raise

    def __exit__(self, exc_type, *_):
        if getattr(self.session, "single_run", False) is True:
            # The private deployment closes immediately after this one-point
            # validation; pausing and deleting its run first is redundant.
            return
        if self.run is not None:
            if exc_type:
                bench.write_json(
                    self.output / "failure.json", self.session.cli("run", "inspect", self.run)
                )
            self.session.cli("run", "pause", self.run)
            self.session.cli(
                "run", "wait", self.run, "--until", "idle", "--timeout", "120s", timeout=125
            )
            self.session.cli("run", "remove", "--yes", self.run)

    def measure(self, point, directory):
        self.session.cli(
            "run",
            "wait",
            self.run,
            "--until",
            "ready",
            "--evaluators",
            self.workers,
            "--max-age",
            "60s",
            "--timeout",
            "60s",
            timeout=65,
        )
        before = self.snapshot()
        start = self.wait(
            lambda s: applied(s, self.settings) and warmed(s, before, point, self.completed_offset),
            seconds=WARMUP_TIMEOUT_SECONDS,
        )
        bench.write_json(
            directory / "transition.json",
            dict(
                fresh_run=self.fresh_start, baseline=before, settled=start, settings=self.settings
            ),
        )
        duration = measurement_seconds(self.suite, point)
        measured = self.session.cli(
            "run",
            "performance",
            self.run,
            "--duration",
            f"{duration}s",
            "--interval",
            f'{self.suite["telemetry_interval_ms"]}ms',
            "--max-age",
            "60s",
            timeout=duration + 20,
        )
        bench.write_json(directory / "observation.json", measured)
        measured = generation_window(measured, point, self.suite, self.completed_offset)
        batches = sum(r["batches_completed"] for r in measured["evaluator_deltas"])
        accepted, _, elapsed = sampler_progress(measured)
        evaluated = sum(r["samples_evaluated"] for r in measured["evaluator_deltas"])
        if measured["valid"] and (
            batches < 8
            or accepted < 8 * point.batch
            or not progress_agrees(point, accepted, evaluated)
        ):
            bench.write_json(directory / "short-interval.json", measured)
            duration = max(duration * 2, min(120, 10 * point.batch / max(1, accepted / elapsed)))
            measured = self.session.cli(
                "run",
                "performance",
                self.run,
                "--duration",
                f"{duration}s",
                "--interval",
                f'{self.suite["telemetry_interval_ms"]}ms',
                "--max-age",
                "60s",
                timeout=duration + 20,
            )
            bench.write_json(directory / "retry-observation.json", measured)
            measured = generation_window(measured, point, self.suite, self.completed_offset)
        bench.write_json(directory / "measurement.json", measured)
        row = measurement_row(measured, point, self.completed_offset)
        return dict(
            row,
            fresh_run=self.fresh_start,
            settings=self.settings,
            run_id=self.run,
            task_id=self.task,
        )


class Measurements:
    def __init__(self, session, output, suite, workers):
        self.session, self.output, self.suite, self.workers = session, output, suite, workers
        self.records = []

    def save(self):
        bench.write_json(self.output / "summary.json", summarize(self.records))

    def has_time(self, point):
        duration = measurement_seconds(self.suite, point)
        return self.session.deadline - time.monotonic() >= max(
            120, duration + 3 * point.batch * point.eval_us / 1e6 + 30
        )

    def measure(self, live, point):
        if not self.has_time(point):
            return None
        directory = self.output / f"point-{len(self.records):03d}"
        directory.mkdir()
        row = dict(point=asdict(point), directory=directory.name, load_average=os.getloadavg())
        bench.write_json(directory / "point.json", row)
        try:
            row.update(live.measure(point, directory))
        except (RuntimeError, TimeoutError, subprocess.TimeoutExpired) as exc:
            row.update(
                valid=False, adequate=False, rate=None, issues=[str(exc)], settings=live.settings
            )
            snapshot = live.snapshot()
            bench.write_json(directory / "failure.json", snapshot)
            if snapshot["failed_tasks"]:
                raise

        self.records.append(row)
        with (self.output / "results.jsonl").open("a") as stream:
            stream.write(json.dumps(row, allow_nan=False) + "\n")
        self.save()
        print(
            f'{directory.name} {point}: {row["rate"]} samples/s valid={row["valid"]} adequate={row["adequate"]}',
            flush=True,
        )
        return row

    def confirm(self, point):
        if not self.has_time(point):
            return None
        directory = self.output / f"confirm-{len(self.records):03d}"
        with LiveRun(self.session, directory, self.suite, point.mode, point.eval_us, point) as live:
            return self.measure(live, point)


def execute(args):
    suite = validate(
        getattr(args, "effective_suite", None) or tomllib.loads(args.suite.read_text())
    )
    if args.budget:
        suite["budget_seconds"] = args.budget
        validate(suite)
    points = (
        [Point(**p) for p in json.loads(args.points.read_text())]
        if args.points
        else sparse_points(suite)
    )
    validate_points(points, suite)
    candidates = bench.physical_cpus()
    if getattr(args, "cpu_limit", None):
        candidates = bench.idle_cpus(candidates, args.cpu_limit)
    loads = bench.core_loads(candidates)
    workers = sorted(suite["workers"])
    infrastructure, evaluator_cpus = worker_cpus(
        candidates, loads, workers, suite["infrastructure_cores"]
    )
    sampler_count = suite.get("sampler_io_threads", 1)
    if sampler_count >= len(infrastructure):
        raise ValueError("infrastructure cores must cover sampler I/O threads plus a database core")
    sampler_cpus = infrastructure[:sampler_count]
    database_cpus = infrastructure[sampler_count:]
    os.sched_setaffinity(0, set(infrastructure + evaluator_cpus))
    for key in [
        "OMP_NUM_THREADS",
        "OPENBLAS_NUM_THREADS",
        "MKL_NUM_THREADS",
        "RAYON_NUM_THREADS",
        "TOKIO_WORKER_THREADS",
    ]:
        os.environ[key] = "1"
    args.output.mkdir(parents=True, exist_ok=False)
    output = args.output.resolve()
    binary, hashes = bench.preserve_inputs(output, args.binary.resolve(strict=True))
    design = "explicit" if args.points else "sparse"
    manifest = dict(
        experiment="frontier",
        schema_version=6,
        workload="batch_sleep",
        timing_jitter=TIMING_JITTER,
        value_coordinate=VALUE_COORDINATE,
        design=design,
        validation_points=[asdict(p) for p in points],
        warmup_timeout_seconds=WARMUP_TIMEOUT_SECONDS,
        minimum_generation_windows=2,
        window_selection="completed_generations_when_observed",
        suite=suite,
        workers=workers,
        evaluator_cpus=evaluator_cpus,
        sampler_cpu=infrastructure[0],
        sampler_cpus=sampler_cpus,
        database_cpus=database_cpus,
        database_directory=str(args.database_directory),
        initial_core_loads=loads,
        physical_evaluator_cores=len(set(evaluator_cpus)),
        skipped_worker_counts=sorted(set(suite["workers"]) - set(workers)),
        binary=str(binary),
        binary_sha256=bench.file_hash(binary),
        harness_files=hashes,
        started_at=datetime.now().isoformat(),
        status="running",
        niceness=os.getpriority(os.PRIO_PROCESS, 0),
        cpu_model=next(
            line.split(":", 1)[1].strip()
            for line in Path("/proc/cpuinfo").read_text().splitlines()
            if line.startswith("model name")
        ),
        rate_source="sampler accepted-sample delta / sampler monotonic telemetry interval",
        scope=f"Shared host; batch sleeps simulate evaluation latency without CPU arithmetic. Evaluator processes share {len(set(evaluator_cpus))} physical cores; no core reservation or exclusive-host ceiling claim.",
        load_average=os.getloadavg(),
    )
    bench.write_json(output / "manifest.json", manifest)
    print(
        f'{design.capitalize()} frontier: workers={workers}, mode={suite["modes"]}, budget={suite["budget_seconds"]}s; shared host load={os.getloadavg()}',
        flush=True,
    )
    start = time.monotonic()
    measurements = None
    try:
        with bench.Session(
            binary,
            output,
            suite["budget_seconds"],
            args.port_offset,
            max_connections=max(128, 4 * (max(workers) + 1) + 32),
            infrastructure_cpus=database_cpus,
            database_directory=args.database_directory,
            shared_buffers=suite.get("database_shared_buffers", "256MB"),
        ) as session:
            session.single_run = len(points) == 1
            session.start_pinned_workers(evaluator_cpus, sampler_cpus)
            measurements = Measurements(session, output, suite, workers)
            jobs = points
            completed_jobs = []
            for job in jobs:
                if not measurements.has_time(job):
                    break
                row = measurements.confirm(job)
                if row and row["valid"] and adequate(row):
                    completed_jobs.append(job)
        manifest["unmeasured_contexts"] = [asdict(j) for j in jobs if j not in completed_jobs]
        manifest["status"] = (
            "completed"
            if len(completed_jobs) == len(jobs)
            else "incomplete" if len(measurements.records) == len(jobs) else "budget_limited"
        )
    except BaseException as exc:
        manifest.update(
            status="incomplete",
            error=str(exc) or type(exc).__name__,
            error_traceback=traceback.format_exc(),
        )
        raise
    finally:
        manifest.update(
            finished_at=datetime.now().isoformat(),
            elapsed_seconds=time.monotonic() - start,
            measured_points=len(measurements.records) if measurements else 0,
        )
        bench.write_json(output / "manifest.json", manifest)
    return output
