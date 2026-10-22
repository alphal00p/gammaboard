from __future__ import annotations

from typing import Any

import numpy as np

from .batches import Batch, GenerationStatus, SampleBatch
from .gammaloop import GammaLoopBatchResult, StratificationContext
from .protocol import (
    PROTOCOL,
    ProtocolIo,
    fixed_domain_shape,
    instantiate_user_object,
    normalize_discrete_subspaces,
    require_homogeneous_offsets,
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
        self.discrete_cardinalities: list[int] | None = None
        self.continuous_dims: int | None = None
        self.components = ["value"]
        self._output_kind: str = "values"

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(self, method: str, params: dict[str, Any], req_binary: bytes = b"") -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "evaluator":
                raise ValueError(f"expected evaluator role, got {params.get('role')!r}")
            discrete_cardinalities, continuous_dims = fixed_domain_shape(params["domain"])
            if any(value <= 0 for value in discrete_cardinalities):
                raise ValueError("discrete_cardinalities must contain only positive integers")
            self._output_kind = str(params["output"]["kind"])
            if self._output_kind not in ("values", "gammaloop"):
                raise ValueError("unknown evaluator output kind")
            if self._output_kind == "values":
                self.components = [str(value) for value in params["output"]["components"]]
                if not self.components:
                    raise ValueError("components must not be empty")
            self.evaluator = instantiate_user_object(
                self.target,
                discrete_cardinalities=discrete_cardinalities,
                continuous_dims=continuous_dims,
                init_args=params.get("args") or {},
            )
            self.discrete_cardinalities = discrete_cardinalities
            self.continuous_dims = continuous_dims
            callback = "eval_gammaloop" if self._output_kind == "gammaloop" else "eval"
            if not callable(getattr(self.evaluator, callback, None)):
                raise TypeError(f"{self._output_kind} evaluator must implement {callback}")
            metadata = _call_optional(self.evaluator, "metadata", {})
            return {"ok": True, "metadata": metadata}
        if self.evaluator is None:
            raise RuntimeError("worker not initialized")
        if method == "diagnostics":
            return {"diagnostics": _call_optional(self.evaluator, "diagnostics", {})}
        if method == "eval_batch":
            if self.evaluator is None or self.discrete_cardinalities is None or self.continuous_dims is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(params["nr_samples"])
            discrete_dims = len(self.discrete_cardinalities)
            require_homogeneous_offsets(params, "xs_discrete_offsets", nr_samples, discrete_dims, "evaluator")
            require_homogeneous_offsets(params, "xs_continuous_offsets", nr_samples, self.continuous_dims, "evaluator")
            coordinate_bytes = nr_samples * (discrete_dims + self.continuous_dims) * 8
            expected_bytes = coordinate_bytes + (nr_samples * 8 if self._output_kind == "gammaloop" else 0)
            if nr_samples < 0 or len(req_binary) != expected_bytes:
                raise ValueError("invalid evaluator input length")
            xs_discrete, xs_continuous = _decode_eval_inputs(
                req_binary, nr_samples, discrete_dims, self.continuous_dims
            )
            self._validate_points(xs_discrete, xs_continuous)
            if self._output_kind == "gammaloop":
                weights = np.frombuffer(req_binary, dtype="<f8", count=nr_samples, offset=coordinate_bytes).copy()
                if not np.isfinite(weights).all() or (weights <= 0).any():
                    raise ValueError("sampling weights must be finite and positive")
                required = params["require_training_values"]
                if not isinstance(required, bool):
                    raise TypeError("require_training_values must be a boolean")
                partition = params["stratification"]
                context = StratificationContext(partition, nr_samples) if partition is not None else None
                return self._eval_batch_gammaloop(xs_discrete, xs_continuous, weights, required, context)
            raw_values = self.evaluator.eval(xs_discrete, xs_continuous)
            values = _normalize_evaluator_values(raw_values, nr_samples, self.components)
            values_row_major = np.ascontiguousarray(
                values.reshape((nr_samples * len(self.components),)), dtype="<f8"
            )
            return {"__binary__": values_row_major.tobytes()}
        raise ValueError(f"unknown method: {method}")

    def _eval_batch_gammaloop(
        self, xs_discrete: np.ndarray, xs_continuous: np.ndarray,
        weights: np.ndarray, require_training_values: bool,
        stratification: StratificationContext | None,
    ) -> dict[str, Any]:
        result = self.evaluator.eval_gammaloop(
            xs_discrete, xs_continuous, weights,
            require_training_values=require_training_values,
            stratification=stratification,
        )
        if not isinstance(result, GammaLoopBatchResult):
            raise TypeError("eval_gammaloop must return GammaLoopBatchResult")
        response: dict[str, Any] = {"gammaloop_state": result.state}
        if require_training_values:
            values = np.asarray(result.training_values, dtype="<f8")
            if values.shape != (len(weights),):
                raise ValueError("GammaLoop training values must contain one weighted value per sample")
            if not np.isfinite(values).all():
                raise ValueError("GammaLoop returned non-finite training feedback")
            response["__binary__"] = values.tobytes()
        elif result.training_values is not None:
            raise ValueError("GammaLoop inference must not return training values")
        if stratification is not None:
            raw = np.asarray(result.values, dtype="<f8")
            # Validate the same classification inputs as the Rust host.
            mask = stratification.mask(raw)
            if require_training_values and (values[~mask] != 0).any():
                raise ValueError("stratum misses must return zero training feedback")
            response["__binary__"] = response.get("__binary__", b"") + np.ascontiguousarray(raw).tobytes()
        elif result.values is not None:
            raise ValueError("GammaLoop values are only requested for stratification")
        return response

    def _validate_points(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> None:
        if not np.isfinite(xs_continuous).all():
            raise ValueError("continuous inputs must be finite")
        for axis, cardinality in enumerate(self.discrete_cardinalities or []):
            axis_values = xs_discrete[:, axis]
            if (axis_values < 0).any() or (axis_values >= cardinality).any():
                raise ValueError(f"xs_discrete axis {axis} out of bounds for cardinality {cardinality}")


class _SamplerWorker:
    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.sampler = None
        self.discrete_cardinalities: list[int] | None = None
        self.continuous_dims: int | None = None
        self.sampler_args: dict[str, Any] = {}
        self.evaluator_metadata: Any = {}

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(self, method: str, params: dict[str, Any], req_binary: bytes = b"") -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "sampler":
                raise ValueError(f"expected sampler role, got {params.get('role')!r}")
            discrete_cardinalities, continuous_dims = fixed_domain_shape(params["domain"])
            if any(value <= 0 for value in discrete_cardinalities):
                raise ValueError("discrete_cardinalities must contain only positive integers")
            self.sampler_args = params.get("args") or {}
            self.evaluator_metadata = params.get("evaluator_metadata") or {}
            restore = getattr(self.target, "from_snapshot", None)
            if not callable(restore) or getattr(restore, "__isabstractmethod__", False):
                raise TypeError("sampler class must implement from_snapshot for recovery")
            initialization = params.get("initialization")
            if not isinstance(initialization, dict):
                raise ValueError("sampler initialization requires a tagged initialization object")
            kind = initialization.get("kind")
            common = dict(discrete_cardinalities=discrete_cardinalities,
                          continuous_dims=continuous_dims, init_args=self.sampler_args,
                          evaluator_metadata=self.evaluator_metadata)
            if kind == "from_stage":
                if initialization.get("source_state") is None or not isinstance(initialization.get("source_args"), dict):
                    raise ValueError("from_stage requires non-null source_state and object source_args")
                handoff = getattr(self.target, "from_stage", None)
                if not callable(handoff):
                    raise TypeError("sampler does not support from_stage")
                self.sampler = handoff(source_state=initialization["source_state"],
                                       source_args=initialization["source_args"], **common)
            elif kind in ("fresh", "restore"):
                snapshot = initialization.get("state")
                if kind == "restore" and snapshot is None:
                    raise ValueError("restore requires non-null state")
                if kind == "fresh" and "state" in initialization:
                    raise ValueError("fresh initialization must not contain state")
                self.sampler = instantiate_user_object(self.target, snapshot=snapshot, **common)
            else:
                raise ValueError(f"unsupported sampler initialization kind: {kind!r}")
            self.discrete_cardinalities = discrete_cardinalities
            self.continuous_dims = continuous_dims
            return {"ok": True}
        if self.sampler is None:
            raise RuntimeError("worker not initialized")
        if method == "generate":
            max_samples = params.get("max_samples")
            if not isinstance(max_samples, int) or isinstance(max_samples, bool) or max_samples <= 0:
                raise ValueError("max_samples must be a positive integer")
            return self._generate(max_samples)
        if method == "feedback":
            nr_values = int(params["nr_values"])
            if nr_values < 0 or len(req_binary) != nr_values * 8:
                raise ValueError("invalid feedback length")
            values = np.frombuffer(req_binary, dtype="<f8", count=nr_values).copy()
            if not np.isfinite(values).all():
                raise ValueError("non-finite training feedback")
            self.sampler.feedback(values)
            return {"ok": True}
        if method == "pdf":
            return self._pdf(params)
        if method == "discrete_pdf":
            return self._discrete_pdf(params)
        if method == "snapshot":
            snapshot = self.sampler.snapshot()
            if snapshot is None:
                raise ValueError("sampler snapshot must not be null; use {} for empty state")
            return {"snapshot": snapshot}
        if method == "diagnostics":
            diagnostics = _call_optional(self.sampler, "diagnostics", {})
            return {"diagnostics": diagnostics if diagnostics is not None else {}}
        if method == "global_pdf_norm":
            norm = float(_call_optional(self.sampler, "global_pdf_norm", 1.0))
            if not np.isfinite(norm) or norm <= 0:
                raise ValueError("global_pdf_norm must be finite and positive")
            return {"norm": norm}
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
        if remaining is not None and (not isinstance(remaining, int) or isinstance(remaining, bool) or remaining < nr_samples):
            raise ValueError("training_remaining must cover the generated samples")
        discrete_dims = len(self.discrete_cardinalities or [])
        continuous_dims = int(self.continuous_dims or 0)
        xs_discrete = np.asarray(batch.xs_discrete, dtype=np.int64).reshape((nr_samples, discrete_dims))
        xs_continuous = np.asarray(batch.xs_continuous, dtype=np.float64).reshape((nr_samples, continuous_dims))
        self._validate_sample_batch(xs_discrete, xs_continuous, weights)
        return {"kind": "batch", "nr_samples": nr_samples, "training_remaining": remaining,
                "__binary__": _encode_batch_binary(xs_discrete, xs_continuous, weights)}

    def _pdf(self, params: dict[str, Any]) -> dict[str, Any]:
        if self.discrete_cardinalities is None or self.continuous_dims is None:
            raise RuntimeError("worker not initialized")
        nr_samples = int(params["nr_samples"])
        discrete_dims = len(self.discrete_cardinalities)
        require_homogeneous_offsets(params, "xs_discrete_offsets", nr_samples, discrete_dims, "sampler")
        require_homogeneous_offsets(params, "xs_continuous_offsets", nr_samples, self.continuous_dims, "sampler")
        xs_discrete = np.asarray(params["xs_discrete_row_major"], dtype=np.int64).reshape((nr_samples, discrete_dims))
        xs_continuous = np.asarray(params["xs_continuous_row_major"], dtype=np.float64).reshape(
            (nr_samples, self.continuous_dims)
        )
        values = _call_optional(self.sampler, "pdf", None, xs_discrete, xs_continuous)
        if values is None:
            return {"values": None}
        if len(values) != nr_samples:
            raise ValueError("pdf must return one value per point")
        return {"values": [None if value is None else float(value) for value in values]}

    def _discrete_pdf(self, params: dict[str, Any]) -> dict[str, Any]:
        subspaces = normalize_discrete_subspaces(params)
        values = _call_optional(self.sampler, "discrete_pdf", None, subspaces)
        if values is None:
            return {"values": None}
        if len(values) != len(subspaces):
            raise ValueError(f"discrete_pdf returned {len(values)} values for {len(subspaces)} subspaces")
        return {"values": [None if value is None else float(value) for value in values]}

    def _validate_sample_batch(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray, weights: np.ndarray) -> None:
        if not np.isfinite(xs_continuous).all():
            raise ValueError("generate returned non-finite continuous values")
        if not np.isfinite(weights).all():
            raise ValueError("generate returned non-finite weights")
        if (weights <= 0.0).any():
            raise ValueError("generate returned non-positive weights")
        for axis, cardinality in enumerate(self.discrete_cardinalities or []):
            axis_values = xs_discrete[:, axis]
            if ((axis_values < 0) | (axis_values >= cardinality)).any():
                raise ValueError(
                    f"generate returned discrete values outside [0, {cardinality}) on axis {axis}"
                )


class _MaterializerWorker:
    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.materializer = None
        self.discrete_cardinalities: list[int] | None = None
        self.continuous_dims: int | None = None

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(self, method: str, params: dict[str, Any], req_binary: bytes = b"") -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "materializer":
                raise ValueError(f"expected materializer role, got {params.get('role')!r}")
            discrete_cardinalities, continuous_dims = fixed_domain_shape(params["domain"])
            if any(value <= 0 for value in discrete_cardinalities):
                raise ValueError("discrete_cardinalities must contain only positive integers")
            self.materializer = instantiate_user_object(
                self.target,
                discrete_cardinalities=discrete_cardinalities,
                continuous_dims=continuous_dims,
                init_args=params.get("args") or {},
            )
            self.discrete_cardinalities = discrete_cardinalities
            self.continuous_dims = continuous_dims
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
        discrete_dims = len(self.discrete_cardinalities or [])
        continuous_dims = int(self.continuous_dims or 0)
        xs_discrete = np.asarray(batch.xs_discrete, dtype=np.int64).reshape((nr_samples, discrete_dims))
        xs_continuous = np.asarray(batch.xs_continuous, dtype=np.float64).reshape((nr_samples, continuous_dims))
        weights = np.asarray(batch.weights, dtype=np.float64).reshape((nr_samples,))
        self._validate_materialized_batch(xs_discrete, xs_continuous, weights)
        return {
            "xs_discrete_row_major": xs_discrete.reshape((nr_samples * discrete_dims,)).tolist(),
            "xs_discrete_offsets": [index * discrete_dims for index in range(nr_samples + 1)],
            "xs_continuous_row_major": xs_continuous.reshape((nr_samples * continuous_dims,)).tolist(),
            "xs_continuous_offsets": [index * continuous_dims for index in range(nr_samples + 1)],
            "weights": weights.tolist(),
        }

    def _validate_materialized_batch(
        self, xs_discrete: np.ndarray, xs_continuous: np.ndarray, weights: np.ndarray
    ) -> None:
        if not np.isfinite(xs_continuous).all():
            raise ValueError("materialize_batch returned non-finite continuous values")
        if not np.isfinite(weights).all():
            raise ValueError("materialize_batch returned non-finite weights")
        if (weights <= 0.0).any():
            raise ValueError("materialize_batch returned non-positive weights")
        for axis, cardinality in enumerate(self.discrete_cardinalities or []):
            axis_values = xs_discrete[:, axis]
            if ((axis_values < 0) | (axis_values >= cardinality)).any():
                raise ValueError(
                    f"materialize_batch returned discrete values outside [0, {cardinality}) on axis {axis}"
                )


class _BatchTransformWorker:
    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.transform = None
        self.discrete_cardinalities: list[int] | None = None
        self.continuous_dims: int | None = None

    def run(self) -> None:
        _serve(self.io, self.handle)

    def handle(self, method: str, params: dict[str, Any], req_binary: bytes = b"") -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "batch_transform":
                raise ValueError(f"expected batch_transform role, got {params.get('role')!r}")
            discrete_cardinalities, continuous_dims = fixed_domain_shape(params["domain"])
            if any(value <= 0 for value in discrete_cardinalities):
                raise ValueError("discrete_cardinalities must contain only positive integers")
            self.transform = instantiate_user_object(
                self.target,
                discrete_cardinalities=discrete_cardinalities,
                continuous_dims=continuous_dims,
                init_args=params.get("args") or {},
            )
            self.discrete_cardinalities = discrete_cardinalities
            self.continuous_dims = continuous_dims
            return {"ok": True}
        if self.transform is None:
            raise RuntimeError("worker not initialized")
        if method == "transform_batch":
            return self._transform_batch(params)
        raise ValueError(f"unknown method: {method}")

    def _transform_batch(self, params: dict[str, Any]) -> dict[str, Any]:
        if self.discrete_cardinalities is None or self.continuous_dims is None:
            raise RuntimeError("worker not initialized")
        nr_samples = int(params["nr_samples"])
        discrete_dims = len(self.discrete_cardinalities)
        require_homogeneous_offsets(params, "xs_discrete_offsets", nr_samples, discrete_dims, "batch_transform")
        require_homogeneous_offsets(params, "xs_continuous_offsets", nr_samples, self.continuous_dims, "batch_transform")
        xs_discrete = np.asarray(params["xs_discrete_row_major"], dtype=np.int64).reshape(
            (nr_samples, discrete_dims)
        )
        xs_continuous = np.asarray(params["xs_continuous_row_major"], dtype=np.float64).reshape(
            (nr_samples, self.continuous_dims)
        )
        weights = np.asarray(params["weights"], dtype=np.float64).reshape((nr_samples,))
        batch = self.transform.transform_batch(xs_discrete, xs_continuous, weights)
        if not isinstance(batch, Batch):
            raise TypeError("transform_batch must return Batch")
        out_discrete = np.asarray(batch.xs_discrete, dtype=np.int64).reshape((nr_samples, discrete_dims))
        out_continuous = np.asarray(batch.xs_continuous, dtype=np.float64).reshape((nr_samples, self.continuous_dims))
        out_weights = np.asarray(batch.weights, dtype=np.float64).reshape((nr_samples,))
        self._validate_transformed_batch(out_discrete, out_continuous, out_weights)
        return {
            "xs_discrete_row_major": out_discrete.reshape((nr_samples * discrete_dims,)).tolist(),
            "xs_discrete_offsets": [index * discrete_dims for index in range(nr_samples + 1)],
            "xs_continuous_row_major": out_continuous.reshape((nr_samples * self.continuous_dims,)).tolist(),
            "xs_continuous_offsets": [index * self.continuous_dims for index in range(nr_samples + 1)],
            "weights": out_weights.tolist(),
        }

    def _validate_transformed_batch(
        self, xs_discrete: np.ndarray, xs_continuous: np.ndarray, weights: np.ndarray
    ) -> None:
        if not np.isfinite(xs_continuous).all():
            raise ValueError("transform_batch returned non-finite continuous values")
        if not np.isfinite(weights).all():
            raise ValueError("transform_batch returned non-finite weights")
        if (weights <= 0.0).any():
            raise ValueError("transform_batch returned non-positive weights")
        for axis, cardinality in enumerate(self.discrete_cardinalities or []):
            axis_values = xs_discrete[:, axis]
            if ((axis_values < 0) | (axis_values >= cardinality)).any():
                raise ValueError(
                    f"transform_batch returned discrete values outside [0, {cardinality}) on axis {axis}"
                )


def _decode_eval_inputs(
    req_binary: bytes, nr_samples: int, discrete_dims: int, continuous_dims: int
) -> tuple[np.ndarray, np.ndarray]:
    """Decode the eval request binary block: little-endian i64 discrete values
    followed by little-endian f64 continuous values (see pack_eval_inputs in
    src/evaluation/evaluator/process.rs). Returns writable arrays."""
    n_discrete = nr_samples * discrete_dims
    n_continuous = nr_samples * continuous_dims
    xs_discrete = np.frombuffer(req_binary, dtype="<i8", count=n_discrete).reshape(
        (nr_samples, discrete_dims)
    )
    xs_continuous = np.frombuffer(
        req_binary, dtype="<f8", count=n_continuous, offset=n_discrete * 8
    ).reshape((nr_samples, continuous_dims))
    # frombuffer views are read-only; hand user code writable copies.
    return np.array(xs_discrete), np.array(xs_continuous)


def _encode_batch_binary(
    xs_discrete: np.ndarray, xs_continuous: np.ndarray, weights: np.ndarray
) -> bytes:
    """Pack a produced batch into the response binary block: little-endian i64
    discrete, then f64 continuous, then f64 weights (see generate in
    src/sampling/sampler_aggregator/process.rs)."""
    # Join contiguous buffer views into one owned frame; do not first copy each
    # array into a separate bytes object. User arrays remain untouched.
    return b"".join(
        (
            np.ascontiguousarray(xs_discrete.reshape(-1), dtype="<i8").data,
            np.ascontiguousarray(xs_continuous.reshape(-1), dtype="<f8").data,
            np.ascontiguousarray(weights.reshape(-1), dtype="<f8").data,
        )
    )


def _normalize_evaluator_values(values: Any, nr_samples: int, components: list[str]) -> np.ndarray:
    if isinstance(values, dict):
        columns = [np.asarray(values[name], dtype=np.float64).reshape((nr_samples,)) for name in components]
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
        raise ValueError(f"sample {sample}, component {components[component]!r}: eval returned non-finite value")
    return array


def _call_optional(obj: Any, name: str, fallback: Any, *args: Any) -> Any:
    method = getattr(obj, name, None)
    if method is None:
        return fallback
    return method(*args)
