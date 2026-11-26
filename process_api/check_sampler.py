#!/usr/bin/env python3
"""Check a process sampler without a GammaBoard server or database.

Use the same executable as your GammaBoard process_sampler command:
  python -B process_api/check_sampler.py ../my_sampler/.venv/bin/my-sampler
  python -B process_api/check_sampler.py ../my_sampler/.venv/bin/my-sampler \
    --init-args '{"seed": 42, "save_path": "/tmp/my-sampler-check"}'
  python -B process_api/check_sampler.py \
    --command '["/path/to/python", "-u", "worker.py"]' --cwd /path/to/worker

Executables use their own interpreter and installed SDK, without a test wrapper.
Commands are argument arrays, never shell strings. The worker inherits the current
directory unless --cwd is supplied; relative executables resolve from that directory.
The checker itself needs NumPy and this checkout's SDK (or an installed SDK).
For Python class checks, module:Class or /path/module.py:Class are also accepted;
these use the checker's interpreter and SDK, importing only in child processes.
Test state stays in memory; Python bytecode and persistent reports are disabled.
The sampler itself may create/modify files.
Commit notifications are simulated; PostgreSQL transaction behavior is not tested.

Defaults: two continuous coordinates, positive toy feedback, exact replay,
128 samples per trajectory, and 60 seconds per RPC. Each recovery scenario
uses separate, sequential processes (no simultaneous copies of a GPU model).
The entire run has a configurable deadline. Executable checks use only the public
protocol: unavailable optional values are SKIP, invalid responses are FAIL. A
successful no-op cannot be distinguished from a custom implementation over RPC.
Class checks additionally inspect optional methods and skip SDK defaults.

Exit codes: 0 = required checks passed, 1 = failed check, 2 = invalid setup or
inconclusive required check, 130 = interrupted. Optional INCONCLUSIVE results
do not fail the run; they are not counted as passes.
"""

from __future__ import annotations

import argparse
import importlib
import importlib.util
import inspect
import itertools
import json
import math
import os
import sys
import time
import uuid
from collections import Counter, deque
from dataclasses import dataclass, field
from pathlib import Path

sys.dont_write_bytecode = True
SDK_SOURCE = Path(__file__).resolve().parent / "python" / "src"
sys.path.insert(0, os.getcwd())
if SDK_SOURCE.is_dir():
    sys.path.insert(0, str(SDK_SOURCE))

sys.path.insert(0,str(Path(__file__).resolve().parent))
from check_common import CheckError, RemoteError, RpcWorker, Unexercised, Unsupported

NOTICE = (
    "No GammaBoard runs, tasks, database records or persistent test reports are created. "
    "Your sampler may create or modify files during initialization, snapshots or cleanup. "
    "Use disposable sampler output paths; you are responsible for cleaning up those artifacts."
)
MAX_FRAME_BYTES = 64 * 1024 * 1024
HOOKS = (
    "feedback",
    "from_stage",
    "checkpoint_committed",
    "pdf",
    "discrete_mass",
    "diagnostics",
    "persisted_output",
)


def load_entry(entry):
    module_name, sep, qualname = entry.rpartition(":")
    if not sep or not module_name or not qualname:
        raise ValueError("entry point must be module:Class or /path/to/module.py:Class")
    if module_name.endswith(".py"):
        path = Path(module_name).resolve()
        sys.path.insert(0, str(path.parent))
        spec = importlib.util.spec_from_file_location("_sampler_under_test", path)
        if spec is None or spec.loader is None:
            raise ValueError(f"cannot import {path}")
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
    else:
        module = importlib.import_module(module_name)
    target = module
    for name in qualname.split("."):
        target = getattr(target, name)
    if not inspect.isclass(target):
        raise TypeError("entry point must refer to a sampler class")
    return target


def worker_main(entry):
    # Install the SDK's normal print redirection before importing user modules.
    from gammaboard_process.abc import Sampler
    from gammaboard_process.log import install_print_redirect
    from gammaboard_process.runners import _SamplerWorker

    install_print_redirect()
    target = load_entry(entry)

    class ProbeWorker(_SamplerWorker):
        def handle(self, method, params, req_binary=b""):
            if method == "_sampler_check_info":
                if self.sampler is None:
                    raise RuntimeError("worker not initialized")
                result = {}
                for name in HOOKS:
                    method = getattr(
                        self.target if name == "from_stage" else self.sampler,
                        name,
                        None,
                    )
                    base = getattr(Sampler, name, None)
                    implementation = getattr(method, "__func__", method)
                    default = getattr(base, "__func__", base)
                    result[name] = (
                        callable(method)
                        and implementation is not default
                        and not getattr(method, "__isabstractmethod__", False)
                    )
                return result
            return super().handle(method, params, req_binary)

    ProbeWorker(target).run()


def validate_snapshot(state, label="snapshot"):
    """Match the host's non-null JSON and exact integer representation limits."""
    if state is None:
        raise CheckError("snapshot must not be None/null")
    json.dumps(state, allow_nan=False)
    pending = [(label, state)]
    while pending:
        path, value = pending.pop()
        if type(value) is int and not -(2**63) <= value <= 2**64 - 1:
            raise CheckError(
                f"{path}: integer exceeds GammaBoard's exact 64-bit JSON range; encode large RNG integers as strings"
            )
        if isinstance(value, dict):
            pending.extend((f"{path}.{key}", item) for key, item in value.items())
        elif isinstance(value, list):
            pending.extend(
                (f"{path}[{index}]", item) for index, item in enumerate(value)
            )


class Worker(RpcWorker):
    def __init__(self, options, init_args, domain, initialization):
        from gammaboard_process.protocol import PROTOCOL
        command=options.command or [sys.executable,"-B",str(Path(__file__).resolve()),"--_worker",options.entry]
        super().__init__(options,command,"sampler",{
            "protocol":PROTOCOL,"role":"sampler","domain":domain.to_dict(),"args":init_args,
            "evaluator_metadata":options.metadata,"initialization":initialization,
        })
        try:
            self.hooks=(self.call("_sampler_check_info",{})[0] if options.command is None else
                {"checkpoint_committed":self.initialized.get("checkpoint_committed") is True})
        except BaseException:
            self.close()
            raise

    def snapshot(self):
        result, binary = self.call("snapshot", {})
        if binary or result.get("snapshot") is None:
            raise CheckError(
                "snapshot must return non-null JSON state without binary data"
            )
        state = result["snapshot"]
        validate_snapshot(state)
        return state

    def acknowledge(self, snapshot):
        if self.initialized.get("checkpoint_committed") is True:
            response, binary = self.call("checkpoint_committed", {"snapshot": snapshot})
            if response.get("ok") is not True or binary:
                raise CheckError("checkpoint_committed must acknowledge with ok=true")



@dataclass
class Event:
    method: str
    params: dict
    binary: bytes
    response: dict
    output: bytes


@dataclass
class Trajectory:
    events: list[Event] = field(default_factory=list)
    samples: int = 0
    feedback_draws: int = 0
    statuses: set[str] = field(default_factory=set)


def decode_draw(domain, limit, result, binary):
    from gammaboard_process.batch_codec import BatchCodec

    kind = result.get("kind")
    if kind in ("waiting", "finished"):
        if binary:
            raise CheckError(f"{kind} response must not carry samples")
        return None
    if kind != "batch":
        raise CheckError(f"unexpected generation status: {kind!r}")
    count = result.get("nr_samples")
    if type(count) is not int or not 1 <= count <= limit:
        raise CheckError(f"batch size {count!r} is outside 1..{limit}")
    remaining = result.get("training_remaining")
    if remaining is not None and (type(remaining) is not int or remaining < count):
        raise CheckError("training_remaining must be an integer covering this draw")
    return BatchCodec(domain).decode(result, binary, weighted=True)


def feedback_request(domain, batch, kind):
    import numpy as np

    # Bounded integrands avoid overflow even for large finite sampling weights.
    values = (
        np.asarray(
            [
                0.5 + 0.25 * (float(np.sin(row).mean()) if len(row) else 0.0)
                for row in batch.xs_continuous
            ]
        )
        * batch.weights
    )
    if kind == "zero":
        values[:] = 0.0
    elif kind == "signed":
        values[1::2] *= -1.0
    if not np.isfinite(values).all():
        raise CheckError("toy feedback overflowed")
    # For an augmented domain, use the requested stratum as the actual stratum.
    # Deliberate cross-stratum masking is outside this synthetic evaluator.
    prefixes = list(bounded_prefixes(domain))
    params = {"nr_values": len(values)}
    binary = np.asarray(values, dtype="<f8").tobytes()
    if prefixes and prefixes != [()]:
        indices = {p: i for i, p in enumerate(prefixes)}
        labels = []
        for row in batch.xs_discrete:
            match = next((p for p in prefixes if tuple(row[: len(p)]) == p), None)
            if match is None:
                raise CheckError("generated sample has no valid stratum prefix")
            labels.append(indices[match])
        params["stratum_prefixes"] = [list(p) for p in prefixes]
        binary += np.asarray(labels + labels, dtype="<u8").tobytes()
    return params, binary


def bounded_prefixes(domain, limit=128):
    from gammaboard_process.domain import DiscreteAxisRole, RectangularDomain

    def rectangular(cards, prefix=()):
        if not cards:
            yield prefix
        else:
            for value in range(cards[0]):
                yield from rectangular(cards[1:], (*prefix, value))

    def visit(node):
        if isinstance(node, RectangularDomain):
            yield from rectangular(node.discrete_cardinalities[: node.strata_dims])
        elif node.axis_role == DiscreteAxisRole.STRATUM:
            for branch in node.branches:
                for suffix in visit(branch.domain):
                    yield (branch.index, *suffix)
        else:
            yield ()

    result = list(itertools.islice(visit(domain), limit + 1))
    if len(result) > limit:
        raise Unexercised(f"test fixture exceeds {limit} stratum prefixes")
    return result


def toy_integrand_domain(domain, prefix):
    """Remove a complete stratum prefix, including ragged tree branches."""
    from gammaboard_process.domain import RectangularDomain

    for depth, label in enumerate(prefix):
        if isinstance(domain, RectangularDomain):
            return RectangularDomain(
                domain.discrete_cardinalities[len(prefix) - depth :],
                domain.continuous_dims,
            )
        domain = next(b.domain for b in domain.branches if b.index == label)
    return domain


def record_call(worker, trajectory, method, params, binary=b""):
    response, output = worker.call(method, params, binary)
    trajectory.events.append(Event(method, params, binary, response, output))
    if method != "generate" and (response.get("ok") is not True or output):
        raise CheckError(f"{method} did not return ok=true")
    return response, output


def drive(
    worker,
    domain,
    options,
    *,
    pending=None,
    max_samples=None,
    max_calls=None,
    allow_empty=False,
):
    trajectory = Trajectory()
    pending = deque(pending or [])
    budget = options.max_samples if max_samples is None else max_samples
    calls = options.max_calls if max_calls is None else max_calls

    def feedback():
        params, binary = pending.popleft()
        record_call(worker, trajectory, "feedback", params, binary)
        trajectory.feedback_draws += 1

    for index in range(calls):
        if trajectory.samples >= budget:
            break
        limit = min(
            options.batch_sizes[index % len(options.batch_sizes)],
            budget - trajectory.samples,
        )
        response, binary = record_call(
            worker, trajectory, "generate", {"max_samples": limit}
        )
        batch = decode_draw(domain, limit, response, binary)
        kind = response["kind"]
        trajectory.statuses.add(kind)
        if kind == "finished":
            break
        if kind == "waiting":
            if not pending:
                raise CheckError(
                    "WAITING with no outstanding feedback: this runner cannot make progress"
                )
            feedback()
            continue
        trajectory.samples += len(batch.weights)
        if response.get("training_remaining") is not None:
            pending.append(feedback_request(domain, batch, options.feedback))
        # Delay feedback to exercise more than one outstanding draw, but preserve FIFO.
        if len(pending) >= 4:
            feedback()
    while pending:
        feedback()
    if trajectory.samples == 0 and not allow_empty:
        raise Unexercised(
            "no sample batch was produced; generation could not be assessed"
        )
    return trajectory


def replay(worker, domain, options, trajectory):
    import numpy as np

    for index, event in enumerate(trajectory.events):
        response, binary = worker.call(event.method, event.params, event.binary)
        if event.method == "generate":
            expected = decode_draw(
                domain, event.params["max_samples"], event.response, event.output
            )
            actual = decode_draw(domain, event.params["max_samples"], response, binary)
            if response.get("kind") != event.response.get("kind"):
                raise CheckError(f"replay call {index}: generation status changed")
            if response.get("training_remaining") != event.response.get(
                "training_remaining"
            ):
                raise CheckError(f"replay call {index}: training_remaining changed")
            if actual is not None:
                for key in ("xs_discrete", "xs_continuous", "weights"):
                    left, right = getattr(expected, key), getattr(actual, key)
                    if len(left) != len(right):
                        raise CheckError(f"replay call {index}: {key} length changed")
                    for a, b in zip(left, right):
                        if np.shape(a) != np.shape(b):
                            raise CheckError(
                                f"replay call {index}: {key} shape changed"
                            )
                        equal = (
                            np.array_equal(a, b)
                            if key == "xs_discrete"
                            else np.allclose(a, b, rtol=options.rtol, atol=options.atol)
                        )
                        if not equal:
                            raise CheckError(
                                f"replay call {index}: {key} changed after restoration"
                            )
        elif response != event.response or binary != event.output:
            raise CheckError(
                f"replay call {index}: {event.method} acknowledgement changed"
            )


@dataclass
class Result:
    name: str
    importance: str
    status: str
    detail: str


class Report:
    def __init__(self):
        self.results = []

    def add(self, name, importance, status, detail):
        self.results.append(Result(name, importance, status, str(detail)))
        print(f"{status:12} [{importance}] {name}: {detail}", flush=True)

    def check(self, name, importance, fn):
        try:
            detail = fn()
            self.add(name, importance, "PASS", detail or "ok")
            return True
        except Unsupported as exc:
            self.add(name, importance, "SKIP", exc)
        except Unexercised as exc:
            self.add(name, importance, "INCONCLUSIVE", exc)
        except Exception as exc:  # noqa: BLE001 - every failed check must appear in the final summary
            self.add(name, importance, "FAIL", f"{type(exc).__name__}: {exc}")
        return False

    def finish(self):
        print("\nSummary")
        print(f"{'Feature':43} {'Requirement':12} Result")
        for result in self.results:
            print(f"{result.name:43} {result.importance:12} {result.status}")
        counts = Counter(result.status for result in self.results)
        print(
            "; ".join(
                f"{counts[status]} {status}"
                for status in ("PASS", "FAIL", "SKIP", "INCONCLUSIVE")
            )
        )
        print("\n" + NOTICE)
        if counts["FAIL"]:
            return 1
        if any(r.importance == "required" and r.status != "PASS" for r in self.results):
            return 2
        return 0


class Suite:
    def __init__(self, options, report):
        self.options, self.report = options, report
        self.domain = options.domain
        self.hooks = {}
        self.training = False

    def worker(self, *, args=None, initialization=None, domain=None):
        return Worker(
            self.options,
            self.options.init_args if args is None else args,
            self.domain if domain is None else domain,
            {"kind": "fresh"} if initialization is None else initialization,
        )

    def restore(self, snapshot, *, args=None):
        return self.worker(
            args=args, initialization={"kind": "restore", "state": snapshot}
        )

    def optional(self, name):
        if self.hooks.get(name) is False:
            raise Unsupported(
                "method absent, SDK default, or capability not advertised"
            )

    def baseline(self):
        try:
            worker = self.worker()
        except Unexercised:
            raise
        except Exception as exc:
            self.report.add(
                "Entry point and initialization", "required", "FAIL", str(exc)
            )
            raise Unexercised("initialization failed") from exc
        with worker:
            self.hooks = worker.hooks
            self.report.add(
                "Entry point and initialization",
                "required",
                "PASS",
                "process sampler initialized via public protocol"
                if self.options.command is not None
                else "Python class initialized in SDK subprocess",
            )
            result = drive(worker, self.domain, self.options, allow_empty=True)
            self.training = result.feedback_draws > 0
            self.report.add(
                "Training feedback and draw ordering",
                "conditional",
                "PASS" if self.training else "SKIP",
                f"{result.feedback_draws} feedback draws"
                if self.training
                else "draws did not request feedback",
            )
            for status in ("waiting", "finished"):
                self.report.add(
                    f"GenerationStatus.{status.upper()}",
                    "optional",
                    "PASS" if status in result.statuses else "INCONCLUSIVE",
                    "handled"
                    if status in result.statuses
                    else "not observed within test budget; not required to occur",
                )
            if result.samples == 0:
                raise Unexercised(
                    "sampler finished without producing samples; generation/recovery could not be assessed"
                )
        limits = sorted(
            {e.params["max_samples"] for e in result.events if e.method == "generate"}
        )
        return f"{result.samples} valid samples; tested generation limits {limits}; clean shutdown"

    def roundtrip(self, *, acknowledge=False, failed_save=False):
        with self.worker() as worker:
            state = worker.snapshot()
            if acknowledge:
                worker.acknowledge(state)
            first = drive(worker, self.domain, self.options)
            if failed_save:
                worker.snapshot()  # Candidate B; deliberately no commit notification.
                second = drive(worker, self.domain, self.options, allow_empty=True)
                first.events.extend(second.events)
        with self.restore(state) as restored:
            replay(restored, self.domain, self.options, first)
        return "continuation matches across a fresh process" + (
            "; A survived unacknowledged B"
            if failed_save
            else "; commit notification was omitted"
        )

    def pending_feedback(self):
        if not self.training:
            raise Unsupported("no training draws observed in baseline")
        with self.worker() as worker:
            initial = worker.snapshot()
            worker.acknowledge(initial)
            response, binary = worker.call(
                "generate", {"max_samples": self.options.batch_sizes[0]}
            )
            batch = decode_draw(
                self.domain, self.options.batch_sizes[0], response, binary
            )
            if batch is None or response.get("training_remaining") is None:
                raise Unexercised("first draw did not request feedback")
            pending = feedback_request(self.domain, batch, self.options.feedback)
            state = worker.snapshot()
            expected = drive(
                worker, self.domain, self.options, pending=[pending], allow_empty=True
            )
        with self.restore(state) as restored:
            replay(restored, self.domain, self.options, expected)
        return "restored pending feedback and subsequent generation match"

    def notifications(self):
        self.optional("checkpoint_committed")
        with self.worker() as worker:
            if worker.initialized.get("checkpoint_committed") is not True:
                raise Unsupported(
                    "SDK does not advertise checkpoint notifications; update the SDK"
                )
            state = worker.snapshot()
            worker.acknowledge(state)
            drive(worker, self.domain, self.options)
            state = worker.snapshot()
            worker.acknowledge(state)
            worker.acknowledge(state)
            expected = drive(worker, self.domain, self.options, allow_empty=True)
        with self.restore(state) as restored:
            replay(restored, self.domain, self.options, expected)
        return (
            "duplicate notifications accepted; latest committed state remains restorable"
            + (
                "; protocol check cannot distinguish a no-op hook"
                if self.options.command is not None
                else ""
            )
        )

    def output(self, method, field):
        self.optional(method)
        with self.worker() as worker:
            values = []
            for after_draws in (False, True):
                if after_draws:
                    drive(worker, self.domain, self.options, allow_empty=True)
                response, binary = worker.call(method, {})
                if field not in response or binary:
                    raise CheckError(f"missing {field} or unexpected binary output")
                json.dumps(response[field], allow_nan=False)
                values.append(response[field])
        if all(value is None for value in values):
            raise Unsupported("callback returned None (unavailable)")
        if self.options.command is not None and all(value == {} for value in values):
            raise Unsupported("empty JSON output; no custom data observed")
        return "JSON-compatible output before and after sampling"

    def pdf(self):
        import numpy as np
        from gammaboard_process.batch_codec import BatchCodec

        self.optional("pdf")
        with self.worker() as worker:
            response, binary = worker.call(
                "generate", {"max_samples": max(self.options.batch_sizes)}
            )
            training = response.get("training_remaining") is not None
            batch = decode_draw(
                self.domain, max(self.options.batch_sizes), response, binary
            )
            if batch is None:
                raise Unexercised("no points available for PDF query")
            params = BatchCodec(self.domain).encode(batch, len(batch.weights))
            params.pop("weights")
            params["nr_samples"] = len(batch.weights)
            response, binary = worker.call("pdf", params)
            values = probability_values(
                response, binary, len(batch.weights), mass=False
            )
            available = [i for i, v in enumerate(values) if v is not None]
            if not available:
                raise Unsupported("all queried densities unavailable")
            if self.options.statistics:
                if training:
                    self.report.add(
                        "PDF / sampling-weight consistency",
                        "optional",
                        "INCONCLUSIVE",
                        "requires inference draws from a frozen proposal",
                    )
                else:
                    products = np.array(
                        [values[i] * batch.weights[i] for i in available]
                    )
                    if not np.allclose(
                        products,
                        1.0,
                        rtol=max(self.options.rtol, 1e-6),
                        atol=max(self.options.atol, 1e-8),
                    ):
                        self.report.add(
                            "PDF / sampling-weight consistency",
                            "optional",
                            "FAIL",
                            f"weight × PDF ranges from {products.min():.6g} to {products.max():.6g}",
                        )
                    else:
                        self.report.add(
                            "PDF / sampling-weight consistency",
                            "optional",
                            "PASS",
                            "weight × PDF agrees with 1 at generated points",
                        )
        return f"{len(available)}/{len(values)} finite nonnegative densities"

    def masses(self):
        from gammaboard_process.domain import RectangularDomain

        if self.hooks.get("discrete_mass") is False:
            if self.options.require_stratified:
                raise CheckError("stratified inference requires discrete_mass")
            self.optional("discrete_mass")
        prefixes = bounded_prefixes(self.domain)
        if self.options.require_stratified and prefixes == [()]:
            raise CheckError(
                "--require-stratified needs a domain with stratum prefixes"
            )
        queries = [{}]
        prefix_start = 1
        if prefixes != [()]:
            queries.extend(dict(enumerate(prefix)) for prefix in prefixes)
        # Exercise ordinary partial assignments even without strata.
        axis_start = len(queries)
        if (
            isinstance(self.domain, RectangularDomain)
            and self.domain.discrete_cardinalities
        ):
            queries.extend(
                {0: index}
                for index in range(min(self.domain.discrete_cardinalities[0], 4))
            )
        wire = [
            {"fixed_dims": [{"dim": d, "value": v} for d, v in query.items()]}
            for query in queries
        ]
        with self.worker() as worker:
            response, binary = worker.call("discrete_mass", {"subspaces": wire})
            values = probability_values(response, binary, len(queries), mass=True)
        if values[0] is not None and not math.isclose(
            values[0], 1, rel_tol=1e-8, abs_tol=1e-10
        ):
            raise CheckError("mass of the unrestricted domain must be 1")
        if prefixes != [()]:
            masses = values[prefix_start : prefix_start + len(prefixes)]
            if self.options.require_stratified and any(v is None for v in masses):
                raise CheckError("complete stratum-prefix masses are unavailable")
            if all(v is not None for v in masses) and not math.isclose(
                sum(masses), 1, rel_tol=1e-8, abs_tol=1e-10
            ):
                raise CheckError(
                    "masses of disjoint exhaustive stratum prefixes do not sum to 1"
                )
        if (
            isinstance(self.domain, RectangularDomain)
            and self.domain.discrete_cardinalities
            and self.domain.discrete_cardinalities[0] <= 4
        ):
            masses = values[axis_start:]
            if all(v is not None for v in masses) and not math.isclose(
                sum(masses), 1, rel_tol=1e-8, abs_tol=1e-10
            ):
                raise CheckError(
                    "masses of the exhaustive first-axis partition do not sum to 1"
                )
        if all(v is None for v in values):
            raise Unsupported("all queried masses unavailable")
        return (
            f"{sum(v is not None for v in values)}/{len(values)} valid marginal masses"
        )

    def stage(self):
        self.optional("from_stage")
        destination = dict(self.options.init_args)
        source_path = destination.get("save_path")
        if isinstance(source_path, str) and source_path:
            destination["save_path"] = (
                source_path.rstrip("/") + ".check-stage-" + uuid.uuid4().hex
            )
        destination.update(self.options.stage_init_args)
        if (
            source_path
            and destination.get("save_path")
            and Path(source_path).resolve() == Path(destination["save_path"]).resolve()
        ):
            raise Unexercised(
                "stage save_path must differ from source; supply --stage-init-args"
            )
        print("Stage init_args:", json.dumps(destination, sort_keys=True), flush=True)
        with self.worker() as worker:
            drive(worker, self.domain, self.options)
            source = worker.snapshot()
            worker.acknowledge(source)
            source_continuation = drive(
                worker, self.domain, self.options, allow_empty=True
            )
        try:
            with self.worker(
                args=destination,
                initialization={
                    "kind": "from_stage",
                    "source_state": source,
                    "source_args": self.options.init_args,
                },
            ) as target:
                state = target.snapshot()
                continuation = drive(target, self.domain, self.options)
        except RemoteError as exc:
            if (
                str(exc).startswith("NotImplementedError:")
                or str(exc).splitlines()[0]
                == "TypeError: sampler does not support from_stage"
            ):
                raise Unsupported(str(exc)) from exc
            raise
        with self.restore(state, args=destination) as restored:
            replay(restored, self.domain, self.options, continuation)
        with self.restore(source) as original:
            replay(original, self.domain, self.options, source_continuation)
        return "destination generates/restores; source continuation remains unchanged"

    def extra_domains(self):
        from gammaboard_process import Domain

        if not self.options.extra_domains:
            self.report.add(
                "Additional domain shapes", "optional", "SKIP", "enable --extra-domains"
            )
            return
        definitions = {
            "discrete-only": {
                "rectangular": {"discrete_cardinalities": [2, 3], "continuous_dims": 0}
            },
            "mixed": {
                "rectangular": {"discrete_cardinalities": [2], "continuous_dims": 2}
            },
            "ragged": {
                "discrete": {
                    "branches": [
                        {
                            "index": 0,
                            "domain": {
                                "rectangular": {
                                    "discrete_cardinalities": [],
                                    "continuous_dims": 1,
                                }
                            },
                        },
                        {
                            "index": 2,
                            "domain": {
                                "rectangular": {
                                    "discrete_cardinalities": [],
                                    "continuous_dims": 3,
                                }
                            },
                        },
                    ]
                }
            },
        }
        for name, definition in definitions.items():
            domain = Domain.from_dict(definition)

            def check(domain=domain):
                try:
                    worker = self.worker(domain=domain)
                except RemoteError as exc:
                    raise Unexercised(
                        f"fixture rejected during initialization: {exc}"
                    ) from exc
                with worker:
                    result = drive(worker, domain, self.options)
                return f"{result.samples} valid samples"

            self.report.check(f"Domain: {name}", "optional", check)

    def statistics(self):
        import numpy as np
        from gammaboard_process.domain import RectangularDomain

        if not self.options.statistics:
            raise Unsupported("enable --statistics for toy-integral smoke checks")
        prefixes = bounded_prefixes(self.domain)
        stratified = prefixes != [()]
        integrand = toy_integrand_domain(self.domain, prefixes[0])
        if not isinstance(integrand, RectangularDomain) or any(
            toy_integrand_domain(self.domain, prefix) != integrand
            for prefix in prefixes
        ):
            raise Unexercised(
                "toy-integral check needs the same rectangular integrand domain under every prefix"
            )
        if stratified and integrand.continuous_dims == 0:
            raise Unexercised("stratified toy integrals need a continuous coordinate")
        volume = math.prod(integrand.discrete_cardinalities)
        count = len(prefixes)
        values = [[] for _ in prefixes]
        matches = [0] * count
        masses = None
        with self.worker() as worker:
            samples = 0
            calls = 0
            while samples < self.options.max_samples and calls < self.options.max_calls:
                limit = min(
                    max(self.options.batch_sizes),
                    self.options.max_samples - samples,
                )
                response, binary = worker.call("generate", {"max_samples": limit})
                calls += 1
                batch = decode_draw(self.domain, limit, response, binary)
                if batch is None:
                    break
                if response.get("training_remaining") is not None:
                    raise Unexercised(
                        "toy-integral statistics require inference draws from a frozen proposal"
                    )
                if any(np.any((row < 0) | (row > 1)) for row in batch.xs_continuous):
                    raise Unexercised("fixture is not sampled on the unit hypercube")
                if stratified and masses is None:
                    queries = [
                        {
                            "fixed_dims": [
                                {"dim": d, "value": v} for d, v in enumerate(p)
                            ]
                        }
                        for p in prefixes
                    ]
                    response, binary = worker.call(
                        "discrete_mass", {"subspaces": queries}
                    )
                    masses = probability_values(response, binary, count, mass=True)
                    if any(p is None for p in masses):
                        raise Unexercised("toy integrals need exact stratum-prefix masses")
                    if any(p <= 0 for p in masses) or not math.isclose(
                        sum(masses), 1, rel_tol=1e-8, abs_tol=1e-10
                    ):
                        raise CheckError(
                            "toy integrals need positive prefix masses summing to 1"
                        )
                for discrete, continuous, weight in zip(
                    batch.xs_discrete, batch.xs_continuous, batch.weights
                ):
                    if not stratified:
                        values[0].append([weight])
                        continue
                    k = next(
                        i
                        for i, p in enumerate(prefixes)
                        if tuple(discrete[: len(p)]) == p
                    )
                    x = continuous[0]
                    hit = min(int(x * count), count - 1) == k
                    matches[k] += int(hit)
                    # Full weights include 1/p_k. Misses remain zero-valued attempts.
                    conditional = masses[k] * weight if hit else 0.0
                    values[k].append([conditional, conditional * (x - 0.5)])
                samples += len(batch.weights)

        details, disagreements = [], []

        def compare(label, estimate, error, expected):
            details.append(
                f"{label}: {estimate:.6g} ± {error:.3g}; expected {expected:.6g}"
            )
            if abs(estimate - expected) > max(6 * error, 1e-8 * volume):
                disagreements.append(label)

        means, variances = [], []
        names = (
            ["integral of 1", "integral of x[0]-1/2"]
            if stratified else ["integral of 1"]
        )
        for k, (prefix, observations) in enumerate(zip(prefixes, values)):
            n = len(observations)
            label = f"stratum {prefix}" if stratified else "inference"
            if n < 32 or (stratified and matches[k] < 2):
                raise Unexercised(
                    f"{label}: N={n}"
                    + (f", matches={matches[k]}" if stratified else "")
                    + "; need at least 32 attempts"
                    + (" and 2 matches per stratum" if stratified else "")
                    + "; increase --max-samples/--max-calls"
                )
            mean = np.mean(observations, axis=0)
            variance = np.var(observations, axis=0, ddof=1) / n
            if not np.isfinite(mean).all() or not np.isfinite(variance).all():
                raise Unexercised("toy-integral moments overflowed")
            means.append(mean)
            variances.append(variance)
            details.append(
                f"{label}: N={n}"
                + (f", matches={matches[k]}, p={masses[k]:.6g}" if stratified else "")
            )
            expected = (
                [volume / count, volume / count * ((k + 0.5) / count - 0.5)]
                if stratified else [volume]
            )
            for name, value, var, exact in zip(names, mean, variance, expected):
                compare(f"{label} {name}", value, math.sqrt(var), exact)
        if stratified:
            # Each conditional estimate already contains p_k; do not weight again.
            for name, value, var, exact in zip(
                names, np.sum(means, axis=0), np.sum(variances, axis=0), [volume, 0]
            ):
                compare(f"combined {name}", value, math.sqrt(var), exact)
        detail = "; ".join(details)
        if disagreements:
            raise Unexercised(
                detail
                + "; disagreement needs investigation: "
                + ", ".join(disagreements)
                + " (statistical evidence, not proof)"
            )
        return detail + "; smoke check only, not a normalization certificate"

    def run(self):
        if not self.report.check(
            "Generation, bounds and shutdown", "required", self.baseline
        ):
            self.report.add(
                "Dependent checks",
                "required",
                "INCONCLUSIVE",
                "baseline must generate valid samples first",
            )
            return
        checks = [
            ("Snapshot JSON and recovery without notice", "required", self.roundtrip),
            (
                "Uncommitted B preserves committed A",
                "required",
                lambda: self.roundtrip(acknowledge=True, failed_save=True),
            ),
            (
                "Recovery with pending feedback",
                "required" if self.training else "conditional",
                self.pending_feedback,
            ),
            ("Checkpoint commit notification", "optional", self.notifications),
            ("PDF return contract", "optional", self.pdf),
            (
                "Discrete mass return contract",
                "required" if self.options.require_stratified else "optional",
                self.masses,
            ),
            (
                "Diagnostics JSON",
                "optional",
                lambda: self.output("diagnostics", "diagnostics"),
            ),
            (
                "Persisted output JSON",
                "optional",
                lambda: self.output("persisted_output", "output"),
            ),
            ("Stage handoff and source preservation", "optional", self.stage),
            ("Toy-integral statistics", "optional", self.statistics),
        ]
        for name, importance, check in checks:
            self.report.check(name, importance, check)
        self.extra_domains()


def probability_values(response, binary, length, *, mass):
    if binary or "values" not in response:
        raise CheckError("probability response must be JSON-only with a values field")
    values = response["values"]
    if values is None:
        return [None] * length
    if not isinstance(values, list) or len(values) != length:
        raise CheckError("probability response length does not match query count")
    for value in values:
        if value is not None and (
            not isinstance(value, (int, float))
            or not math.isfinite(value)
            or value < 0
            or (mass and value > 1)
        ):
            raise CheckError("probability response contains an invalid density/mass")
    return values


def json_object(value):
    try:
        parsed = json.loads(
            Path(value[1:]).read_text() if value.startswith("@") else value
        )
        if not isinstance(parsed, dict):
            raise TypeError("expected a JSON object")
        validate_snapshot(parsed, label="JSON argument")
        return parsed
    except (ValueError, TypeError, CheckError, OSError) as exc:
        raise argparse.ArgumentTypeError(str(exc)) from exc


def json_command(value):
    try:
        command = json.loads(value)
        if (
            not isinstance(command, list)
            or not command
            or any(not isinstance(arg, str) or "\x00" in arg for arg in command)
            or not command[0].strip()
        ):
            raise ValueError(
                "command must be a nonempty JSON array of strings with an executable first"
            )
        return command
    except ValueError as exc:
        raise argparse.ArgumentTypeError(str(exc)) from exc


def parse_args(argv):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "entry",
        nargs="?",
        help="sampler executable, module:Class or /path/module.py:Class",
    )
    parser.add_argument(
        "--command",
        type=json_command,
        help='GammaBoard command as a JSON argument array, e.g. ["python", "-u", "worker.py"]; alternative to entry',
    )
    parser.add_argument(
        "--cwd",
        help="worker working directory (default: current directory); use the same directory as your run config",
    )
    parser.add_argument(
        "--init-args",
        type=json_object,
        default={},
        help="JSON object or @file; overlays per-case constructor defaults",
    )
    parser.add_argument(
        "--stage-init-args",
        type=json_object,
        default={},
        help="destination overrides; defaults to source args with a unique save_path suffix",
    )
    parser.add_argument(
        "--domain",
        type=json_object,
        help="SDK domain JSON or @file; defaults to two continuous dimensions",
    )
    parser.add_argument(
        "--evaluator-metadata",
        dest="metadata",
        type=json_object,
        default={},
        help="metadata JSON or @file",
    )
    parser.add_argument(
        "--max-samples",
        type=int,
        default=128,
        help="maximum samples per trajectory (replay scenarios run several trajectories)",
    )
    parser.add_argument(
        "--max-calls",
        type=int,
        default=64,
        help="maximum generate calls per trajectory",
    )
    parser.add_argument(
        "--batch-sizes",
        default="1,7,3,16",
        help="comma-separated positive generation limits",
    )
    parser.add_argument(
        "--timeout", type=float, default=60, help="seconds per RPC, including startup"
    )
    parser.add_argument(
        "--time-limit", type=float, default=300, help="overall time budget in seconds"
    )
    parser.add_argument(
        "--rtol",
        type=float,
        default=0,
        help="explicit relative tolerance for restored floating-point samples",
    )
    parser.add_argument(
        "--atol",
        type=float,
        default=0,
        help="explicit absolute tolerance for restored floating-point samples",
    )
    parser.add_argument(
        "--feedback", choices=("positive", "zero", "signed"), default="positive"
    )
    parser.add_argument(
        "--require-stratified",
        action="store_true",
        help="require complete prefix masses; default domain becomes two strata",
    )
    parser.add_argument(
        "--extra-domains",
        action="store_true",
        help="also try discrete-only, mixed and ragged domains",
    )
    parser.add_argument(
        "--statistics",
        action="store_true",
        help="separate toy-integral and PDF/weight smoke checks",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="include worker callback traceback details",
    )
    options = parser.parse_args(argv)
    if (options.entry is None) == (options.command is None):
        parser.error("supply either an entry point or --command")
    # Retain class-mode compatibility. --command also accepts executables
    # whose literal filenames contain a colon, without any guessing.
    if options.entry is not None and ":" not in options.entry:
        if not options.entry.strip() or "\x00" in options.entry:
            parser.error("entry point must not be empty or contain NUL")
        options.command = [options.entry]
    if options.cwd is not None:
        options.cwd = str(Path(options.cwd).resolve())
        if not Path(options.cwd).is_dir():
            parser.error("--cwd must be an existing directory")
    try:
        from gammaboard_process import Domain

        options.batch_sizes = [int(value) for value in options.batch_sizes.split(",")]
        if (
            not options.batch_sizes
            or min(options.batch_sizes) <= 0
            or min(options.max_samples, options.max_calls) <= 0
        ):
            raise ValueError(
                "sample/call budgets and generation limits must be positive"
            )
        if any(
            not math.isfinite(v) or v <= 0
            for v in (options.timeout, options.time_limit)
        ):
            raise ValueError("timeouts must be finite and positive")
        if any(not math.isfinite(v) or v < 0 for v in (options.rtol, options.atol)):
            raise ValueError("replay tolerances must be finite and nonnegative")
        for args in (options.init_args, options.stage_init_args):
            if {"domain", "evaluator_metadata"}.intersection(args):
                raise ValueError(
                    "domain/evaluator_metadata are reserved; use their dedicated options"
                )
        options.domain = Domain.from_dict(
            options.domain
            or {
                "rectangular": {
                    "discrete_cardinalities": [2] if options.require_stratified else [],
                    "continuous_dims": 2,
                    "strata_dims": 1 if options.require_stratified else 0,
                }
            }
        )
        bounded_prefixes(options.domain)
    except (ValueError, TypeError, KeyError, ImportError, Unexercised) as exc:
        parser.error(str(exc))
    options.deadline = time.monotonic() + options.time_limit
    return options


def main(argv=None):
    options = parse_args(argv)
    report = Report()
    print(NOTICE)
    print(
        "Entry point:",
        json.dumps(options.command) if options.command is not None else options.entry,
    )
    print("Worker directory:", options.cwd or os.getcwd())
    if options.command is not None:
        print(
            "Executable mode: optional features are checked through the public protocol; Python overrides cannot be inspected."
        )
    print("Domain:", json.dumps(options.domain.to_dict(), sort_keys=True))
    print("Init args:", json.dumps(options.init_args, sort_keys=True))
    print(
        f"Replay tolerances: rtol={options.rtol:g}, atol={options.atol:g}; feedback={options.feedback}",
        flush=True,
    )
    interrupted = False
    try:
        Suite(options, report).run()
    except KeyboardInterrupt:
        interrupted = True
        report.add(
            "Interrupted", "required", "INCONCLUSIVE", "sampler processes stopped"
        )
    except Exception as exc:  # noqa: BLE001 - preserve a summary and cleanup notice on runner failures
        report.add("Test runner", "required", "FAIL", f"{type(exc).__name__}: {exc}")
    code = report.finish()
    return 130 if interrupted else code


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--_worker":
        worker_main(sys.argv[2])
    else:
        raise SystemExit(main())
