from __future__ import annotations

from typing import Any

import numpy as np

from .batch_codec import BatchCodec
from .domain import Domain
from .feedback import Feedback, StratumAssignments
from .batches import Batch, GenerationStatus, SampleBatch
from .gammaloop import GammaLoopBatchResult, StratificationContext
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
    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.evaluator = None
        self.codec: BatchCodec | None = None
        self.components = ["value"]
        self._output_kind: str = "values"

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(
        self, method: str, params: dict[str, Any], req_binary: bytes = b""
    ) -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "evaluator":
                raise ValueError(f"expected evaluator role, got {params.get('role')!r}")
            domain = Domain.from_dict(params["domain"])
            self.codec = BatchCodec(domain)
            self._output_kind = str(params["output"]["kind"])
            if self._output_kind not in ("values", "gammaloop"):
                raise ValueError("unknown evaluator output kind")
            if self._output_kind == "values":
                self.components = [
                    str(value) for value in params["output"]["components"]
                ]
                if not self.components:
                    raise ValueError("components must not be empty")
            self.evaluator = instantiate_user_object(
                self.target,
                domain=domain,
                init_args=params.get("args") or {},
            )
            callback = "eval_gammaloop" if self._output_kind == "gammaloop" else "eval"
            if not callable(getattr(self.evaluator, callback, None)):
                raise TypeError(
                    f"{self._output_kind} evaluator must implement {callback}"
                )
            metadata = _call_optional(self.evaluator, "metadata", {})
            return {"ok": True, "metadata": metadata}
        if self.evaluator is None:
            raise RuntimeError("worker not initialized")
        if method == "diagnostics":
            return {"diagnostics": _call_optional(self.evaluator, "diagnostics", {})}
        if method == "eval_batch":
            nr_samples = params["nr_samples"]
            batch = self.codec.decode(
                params, req_binary, weighted=self._output_kind == "gammaloop"
            )
            xs_discrete, xs_continuous, weights = (
                batch.xs_discrete,
                batch.xs_continuous,
                batch.weights,
            )
            if self._output_kind == "gammaloop":
                required = params["require_training_values"]
                if not isinstance(required, bool):
                    raise TypeError("require_training_values must be a boolean")
                partition = params["stratification"]
                context = (
                    StratificationContext(partition, nr_samples)
                    if partition is not None
                    else None
                )
                return self._eval_batch_gammaloop(
                    xs_discrete, xs_continuous, weights, required, context
                )
            raw_values = self.evaluator.eval(xs_discrete, xs_continuous)
            values = _normalize_evaluator_values(
                raw_values, nr_samples, self.components
            )
            values_row_major = np.ascontiguousarray(
                values.reshape((nr_samples * len(self.components),)), dtype="<f8"
            )
            return {"__binary__": values_row_major.tobytes()}
        raise ValueError(f"unknown method: {method}")

    def _eval_batch_gammaloop(
        self,
        xs_discrete: np.ndarray,
        xs_continuous: np.ndarray,
        weights: np.ndarray,
        require_training_values: bool,
        stratification: StratificationContext | None,
    ) -> dict[str, Any]:
        result = self.evaluator.eval_gammaloop(
            xs_discrete,
            xs_continuous,
            weights,
            require_training_values=require_training_values,
            stratification=stratification,
        )
        if not isinstance(result, GammaLoopBatchResult):
            raise TypeError("eval_gammaloop must return GammaLoopBatchResult")
        response: dict[str, Any] = {"gammaloop_state": result.state}
        if require_training_values:
            values = np.asarray(result.training_values, dtype="<f8")
            if values.shape != (len(weights),):
                raise ValueError(
                    "GammaLoop training values must contain one weighted value per sample"
                )
            if not np.isfinite(values).all():
                raise ValueError("GammaLoop returned non-finite training feedback")
            response["__binary__"] = values.tobytes()
        elif result.training_values is not None:
            raise ValueError("GammaLoop inference must not return training values")
        if stratification is not None:
            raw = np.asarray(result.values, dtype="<f8")
            # Validate the same classification inputs as the Rust host.
            stratification.mask(raw)
            response["__binary__"] = (
                response.get("__binary__", b"") + np.ascontiguousarray(raw).tobytes()
            )
        elif result.values is not None:
            raise ValueError("GammaLoop values are only requested for stratification")
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
            return {"ok": True}
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


def _normalize_evaluator_values(
    values: Any, nr_samples: int, components: list[str]
) -> np.ndarray:
    if isinstance(values, dict):
        columns = [
            np.asarray(values[name], dtype=np.float64).reshape((nr_samples,))
            for name in components
        ]
        values = np.stack(columns, axis=1)
    array = np.asarray(values, dtype=np.float64)
    if len(components) == 1 and array.shape == (nr_samples,):
        array = array.reshape((nr_samples, 1))
    if array.shape != (nr_samples, len(components)):
        raise ValueError(
            f"eval returned shape {array.shape}, expected ({nr_samples},) or ({nr_samples}, {len(components)})"
        )
    finite = np.isfinite(array)
    if not finite.all():
        sample, component = np.argwhere(~finite)[0]
        raise ValueError(
            f"sample {sample}, component {components[component]!r}: eval returned non-finite value"
        )
    return array


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
