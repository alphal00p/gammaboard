from __future__ import annotations

from typing import Any

import numpy as np

from .batch_codec import BatchCodec
from .batches import Batch, GenerationStatus, SampleBatch
from .domain import Domain
from .evaluation import (
    BINARY_ENCODING,
    EVALUATION_PROTOCOL,
    EvaluationBatch,
    EvaluationEncoder,
    EvaluationRequest,
    EvaluatorDescriptor,
    json_response,
    normalize_values,
    validate_batch,
)
from .feedback import Feedback, StratumAssignments
from .protocol import (
    PROTOCOL,
    ProtocolIo,
    instantiate_user_object,
    normalize_discrete_subspaces,
)


def run_evaluator(target: Any) -> None:
    worker = _EvaluatorWorker(target)
    worker.run()


def run_sampler(target: Any) -> None:
    worker = _SamplerWorker(target)
    worker.run()


def run_materializer(target: Any) -> None:
    worker = _MaterializerWorker(target)
    worker.run()


def run_batch_transform(target: Any) -> None:
    worker = _BatchTransformWorker(target)
    worker.run()


def _serve(io: ProtocolIo, handle: Any) -> None:
    """Frame loop shared by all worker roles.

    `handle(method, params, req_binary)` returns a JSON-serializable dict; a
    `__binary__` bytes entry (if present) is sent as the frame's binary block.
    """
    while True:
        frame = io.read_frame()
        if frame is None:
            break
        req, req_binary = frame
        req_id = req.get("id")
        try:
            result = handle(req["method"], req.get("params") or {}, req_binary)
            binary = result.pop("__binary__", b"") if isinstance(result, dict) else b""
            io.send_result(req_id, result, binary)
        except Exception as exc:
            io.send_error(req_id, exc)


class _EvaluatorWorker:
    """One selected-output API, with a v10 adapter for existing evaluators/hosts."""

    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.evaluator = None
        self.codec = None
        self.descriptor = None
        self.encoder = EvaluationEncoder()
        self.modern = False
        self.components = ["value"]
        self._output_kind = "values"

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(self, method, params, req_binary=b""):
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "evaluator":
                raise ValueError(f"expected evaluator role, got {params.get('role')!r}")
            domain = (
                Domain.from_dict(params["domain"]) if params.get("domain") else None
            )
            output = params.get("output") or {"kind": "values", "components": ["value"]}
            self._output_kind = output["kind"]
            if self._output_kind not in ("values", "gammaloop"):
                raise ValueError("unknown evaluator output kind")
            self.components = (
                ["real", "imag"]
                if self._output_kind == "gammaloop"
                else output["components"]
            )
            self.evaluator = instantiate_user_object(
                self.target, domain=domain, init_args=params.get("args") or {}
            )
            descriptor = _call_optional(self.evaluator, "descriptor", None)
            self.modern = descriptor is not None
            if isinstance(descriptor, dict):
                descriptor = EvaluatorDescriptor(**descriptor)
            if self.modern:
                if not isinstance(descriptor, EvaluatorDescriptor):
                    raise TypeError("descriptor must return EvaluatorDescriptor")
                if domain is not None and domain != descriptor.domain:
                    raise ValueError(
                        "configured domain does not match evaluator descriptor"
                    )
            else:
                if domain is None:
                    raise ValueError("legacy evaluators require an explicit domain")
                descriptor = EvaluatorDescriptor(
                    domain,
                    self.components,
                    observables=self._output_kind == "gammaloop",
                )
            self.descriptor = descriptor
            self.codec = BatchCodec(descriptor.domain)
            self.encoder = EvaluationEncoder()
            response = {
                "ok": True,
                "metadata": _call_optional(self.evaluator, "metadata", {}),
            }
            if self.modern:
                response.update(
                    evaluation_protocol=EVALUATION_PROTOCOL,
                    descriptor=descriptor.to_dict(),
                    evaluation_encodings=[BINARY_ENCODING, "json"],
                )
            return response
        if self.evaluator is None:
            raise RuntimeError("worker not initialized")
        if method == "diagnostics":
            return {"diagnostics": _call_optional(self.evaluator, "diagnostics", {})}
        if method not in ("evaluate", "eval_batch"):
            raise ValueError(f"unknown method: {method}")
        if method == "evaluate":
            request = EvaluationRequest(**params["outputs"])
            encoding = params.get("encoding", "json")
            if encoding not in ("json", BINARY_ENCODING):
                raise ValueError("unsupported evaluation encoding")
        else:
            request = EvaluationRequest(
                self.components, observables=self._output_kind == "gammaloop"
            )
        request.validate(self.descriptor)
        batch = self.codec.decode(params, req_binary, weighted=False)
        result = (
            self.evaluator.evaluate(batch.xs_discrete, batch.xs_continuous, request)
            if self.modern
            else self.evaluator.eval(batch.xs_discrete, batch.xs_continuous)
        )
        if not isinstance(result, EvaluationBatch):
            if self.modern or request.observables:
                raise TypeError(
                    "evaluator must return EvaluationBatch (ObservableBatch for legacy workers)"
                )
            result = EvaluationBatch(result)
        values = normalize_values(
            result.values, params["nr_samples"], request.components
        )
        if method == "evaluate":
            return (
                self.encoder.encode(result, request, values)
                if encoding == BINARY_ENCODING
                else json_response(result, request, values)
            )
        validate_batch(result, request, len(values))
        response = {"__binary__": np.ascontiguousarray(values, dtype="<f8").tobytes()}
        if request.observables:
            response["observables"] = result.observables
        return response


class _SamplerWorker:
    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.sampler = None
        self.codec: BatchCodec | None = None
        self.sampler_args: dict[str, Any] = {}
        self.evaluator_metadata: Any = {}

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(
        self, method: str, params: dict[str, Any], req_binary: bytes = b""
    ) -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "sampler":
                raise ValueError(f"expected sampler role, got {params.get('role')!r}")
            domain = Domain.from_dict(params["domain"])
            self.codec = BatchCodec(domain)
            self.sampler_args = params.get("args") or {}
            self.evaluator_metadata = params.get("evaluator_metadata") or {}
            restore = getattr(self.target, "from_snapshot", None)
            if not callable(restore) or getattr(restore, "__isabstractmethod__", False):
                raise TypeError(
                    "sampler class must implement from_snapshot for recovery"
                )
            initialization = params.get("initialization")
            if not isinstance(initialization, dict):
                raise ValueError(
                    "sampler initialization requires a tagged initialization object"
                )
            kind = initialization.get("kind")
            common = dict(
                domain=domain,
                init_args=self.sampler_args,
                evaluator_metadata=self.evaluator_metadata,
            )
            if kind == "from_stage":
                if initialization.get("source_state") is None or not isinstance(
                    initialization.get("source_args"), dict
                ):
                    raise ValueError(
                        "from_stage requires non-null source_state and object source_args"
                    )
                handoff = getattr(self.target, "from_stage", None)
                if not callable(handoff):
                    raise TypeError("sampler does not support from_stage")
                self.sampler = handoff(
                    source_state=initialization["source_state"],
                    source_args=initialization["source_args"],
                    **common,
                )
            elif kind in ("fresh", "restore"):
                snapshot = initialization.get("state")
                if kind == "restore" and snapshot is None:
                    raise ValueError("restore requires non-null state")
                if kind == "fresh" and "state" in initialization:
                    raise ValueError("fresh initialization must not contain state")
                self.sampler = instantiate_user_object(
                    self.target, snapshot=snapshot, **common
                )
            else:
                raise ValueError(f"unsupported sampler initialization kind: {kind!r}")
            return {"ok": True, "checkpoint_committed": True}
        if self.sampler is None:
            raise RuntimeError("worker not initialized")
        if method == "generate":
            max_samples = params.get("max_samples")
            if (
                not isinstance(max_samples, int)
                or isinstance(max_samples, bool)
                or max_samples <= 0
            ):
                raise ValueError("max_samples must be a positive integer")
            return self._generate(max_samples)
        if method == "feedback":
            nr_values = int(params["nr_values"])
            prefixes = params.get("stratum_prefixes")
            if nr_values < 0 or len(req_binary) != nr_values * (
                24 if prefixes is not None else 8
            ):
                raise ValueError("invalid feedback length")
            values = np.frombuffer(req_binary, dtype="<f8", count=nr_values).copy()
            strata = None
            if prefixes is not None:
                labels = np.frombuffer(
                    req_binary, dtype="<u8", count=2 * nr_values, offset=8 * nr_values
                ).copy()
                strata = StratumAssignments(
                    tuple(tuple(p) for p in prefixes),
                    labels[:nr_values],
                    labels[nr_values:],
                )
            self.sampler.feedback(Feedback(values, strata))
            return {"ok": True}
        if method == "pdf":
            return self._pdf(params)
        if method == "discrete_mass":
            return self._discrete_mass(params)
        if method == "snapshot":
            snapshot = self.sampler.snapshot()
            if snapshot is None:
                raise ValueError(
                    "sampler snapshot must not be null; use {} for empty state"
                )
            return {"snapshot": snapshot}
        if method == "checkpoint_committed":
            snapshot = params.get("snapshot")
            if snapshot is None:
                raise ValueError("checkpoint_committed requires non-null snapshot")
            callback = getattr(self.sampler, "checkpoint_committed", None)
            if callback is not None:
                callback(snapshot=snapshot)
            return {"ok": True}
        if method == "diagnostics":
            diagnostics = _call_optional(self.sampler, "diagnostics", {})
            return {"diagnostics": diagnostics if diagnostics is not None else {}}
        if method == "persisted_output":
            return {"output": _call_optional(self.sampler, "persisted_output", None)}
        raise ValueError(f"unknown method: {method}")

    def _generate(self, max_samples: int) -> dict[str, Any]:
        generated = self.sampler.generate(max_samples)
        if isinstance(generated, GenerationStatus):
            return {"kind": generated.value}
        batch = generated
        if not isinstance(batch, SampleBatch):
            raise TypeError("generate must return SampleBatch, WAITING or FINISHED")
        weights = np.asarray(batch.weights, dtype=np.float64)
        if weights.ndim != 1 or len(weights) == 0 or len(weights) > max_samples:
            raise ValueError("generated sample count is empty or exceeds max_samples")
        nr_samples = len(weights)
        remaining = batch.training_remaining
        if remaining is not None and (
            not isinstance(remaining, int)
            or isinstance(remaining, bool)
            or remaining < nr_samples
        ):
            raise ValueError("training_remaining must cover the generated samples")
        return {
            "kind": "batch",
            "nr_samples": nr_samples,
            "training_remaining": remaining,
            **self.codec.encode(batch, nr_samples, binary=True),
        }

    def _pdf(self, params: dict[str, Any]) -> dict[str, Any]:
        nr_samples = params["nr_samples"]
        batch = self.codec.decode(params)
        xs_discrete, xs_continuous = batch.xs_discrete, batch.xs_continuous
        values = _call_optional(self.sampler, "pdf", None, xs_discrete, xs_continuous)
        return {"values": _probabilities(values, nr_samples, mass=False)}

    def _discrete_mass(self, params: dict[str, Any]) -> dict[str, Any]:
        subspaces = normalize_discrete_subspaces(params)
        values = _call_optional(self.sampler, "discrete_mass", None, subspaces)
        return {"values": _probabilities(values, len(subspaces), mass=True)}


class _MaterializerWorker:
    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.materializer = None
        self.codec: BatchCodec | None = None

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(
        self, method: str, params: dict[str, Any], req_binary: bytes = b""
    ) -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "materializer":
                raise ValueError(
                    f"expected materializer role, got {params.get('role')!r}"
                )
            domain = Domain.from_dict(params["domain"])
            self.codec = BatchCodec(domain)
            self.materializer = instantiate_user_object(
                self.target,
                domain=domain,
                init_args=params.get("args") or {},
            )
            return {"ok": True}
        if self.materializer is None:
            raise RuntimeError("worker not initialized")
        if method == "materialize_batch":
            return self._materialize_batch(params)
        raise ValueError(f"unknown method: {method}")

    def _materialize_batch(self, params: dict[str, Any]) -> dict[str, Any]:
        nr_samples = int(params["nr_samples"])
        batch = self.materializer.materialize_batch(params["latent_batch"])
        if not isinstance(batch, Batch):
            raise TypeError("materialize_batch must return Batch")
        return self.codec.encode(batch, nr_samples)


class _BatchTransformWorker:
    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.transform = None
        self.codec: BatchCodec | None = None

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(
        self, method: str, params: dict[str, Any], req_binary: bytes = b""
    ) -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "batch_transform":
                raise ValueError(
                    f"expected batch_transform role, got {params.get('role')!r}"
                )
            domain = Domain.from_dict(params["domain"])
            self.codec = BatchCodec(domain)
            self.transform = instantiate_user_object(
                self.target,
                domain=domain,
                init_args=params.get("args") or {},
            )
            return {"ok": True}
        if self.transform is None:
            raise RuntimeError("worker not initialized")
        if method == "transform_batch":
            return self._transform_batch(params)
        raise ValueError(f"unknown method: {method}")

    def _transform_batch(self, params: dict[str, Any]) -> dict[str, Any]:
        nr_samples = params["nr_samples"]
        incoming = self.codec.decode(params, weighted=True)
        batch = self.transform.transform_batch(
            incoming.xs_discrete, incoming.xs_continuous, incoming.weights
        )
        return self.codec.encode(batch, nr_samples)


_normalize_evaluator_values = normalize_values


def _call_optional(obj: Any, name: str, fallback: Any, *args: Any) -> Any:
    method = getattr(obj, name, None)
    if method is None:
        return fallback
    return method(*args)


def _probabilities(values, size, *, mass):
    if values is None:
        return None
    if len(values) != size:
        raise ValueError("probability query response size mismatch")
    result = [None if v is None else float(v) for v in values]
    if any(
        v is not None and (not np.isfinite(v) or v < 0 or (mass and v > 1))
        for v in result
    ):
        raise ValueError("invalid normalized probability result")
    return result
