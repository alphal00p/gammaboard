from __future__ import annotations

from typing import Any

import numpy as np

from .batches import SampleBatch
from .gammaloop import GammaLoopBatchResult
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


class _EvaluatorWorker:
    def __init__(self, target: Any) -> None:
        self.target = target
        self.io = ProtocolIo()
        self.evaluator = None
        self.discrete_cardinalities: list[int] | None = None
        self.continuous_dims: int | None = None
        self.components = ["value"]
        self._accumulator_kind: str = "vector"

    def run(self) -> None:
        while True:
            req = self.io.read_frame()
            if req is None:
                break
            req_id = req.get("id")
            try:
                result = self.handle(req["method"], req.get("params") or {})
                self.io.send_result(req_id, result)
            except Exception as exc:
                self.io.send_error(req_id, exc)

    def handle(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        if method == "initialize":
            if params.get("protocol") != PROTOCOL:
                raise ValueError(f"unsupported protocol: {params.get('protocol')!r}")
            if params.get("role") != "evaluator":
                raise ValueError(f"expected evaluator role, got {params.get('role')!r}")
            discrete_cardinalities, continuous_dims = fixed_domain_shape(params["domain"])
            if any(value <= 0 for value in discrete_cardinalities):
                raise ValueError("discrete_cardinalities must contain only positive integers")
            self._accumulator_kind = str(params.get("accumulator") or "vector")
            if self._accumulator_kind != "gammaloop":
                self.components = [str(value) for value in params.get("components", ["value"])]
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
            metadata = _read_metadata(self.evaluator)
            return {"ok": True, "metadata": metadata}
        if method == "eval_batch":
            if self.evaluator is None or self.discrete_cardinalities is None or self.continuous_dims is None:
                raise RuntimeError("worker not initialized")
            nr_samples = int(params["nr_samples"])
            discrete_dims = len(self.discrete_cardinalities)
            require_homogeneous_offsets(params, "xs_discrete_offsets", nr_samples, discrete_dims, "evaluator")
            require_homogeneous_offsets(params, "xs_continuous_offsets", nr_samples, self.continuous_dims, "evaluator")
            xs_discrete = np.asarray(params["xs_discrete_row_major"], dtype=np.int64).reshape(
                (nr_samples, discrete_dims)
            )
            xs_continuous = np.asarray(params["xs_continuous_row_major"], dtype=np.float64).reshape(
                (nr_samples, self.continuous_dims)
            )
            self._validate_points(xs_discrete, xs_continuous)
            if self._accumulator_kind == "gammaloop":
                return self._eval_batch_gammaloop(xs_discrete, xs_continuous, nr_samples)
            raw_values = self.evaluator.eval(xs_discrete, xs_continuous)
            values = _normalize_evaluator_values(raw_values, nr_samples, self.components)
            return {"values_row_major": values.reshape((nr_samples * len(self.components),)).tolist()}
        raise ValueError(f"unknown method: {method}")

    def _eval_batch_gammaloop(
        self,
        xs_discrete: np.ndarray,
        xs_continuous: np.ndarray,
        nr_samples: int,
    ) -> dict[str, Any]:
        raw = self.evaluator.eval_gammaloop(xs_discrete, xs_continuous)
        result = _normalize_gammaloop_result(raw, nr_samples)
        response: dict[str, Any] = {"gammaloop_state": result.state}
        if result.training_values is not None:
            tvs = list(result.training_values)
            if len(tvs) != nr_samples:
                raise ValueError(
                    f"GammaLoopBatchResult.training_values length {len(tvs)} "
                    f"does not match nr_samples {nr_samples}"
                )
            response["training_values"] = [float(v) for v in tvs]
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
        while True:
            req = self.io.read_frame()
            if req is None:
                break
            req_id = req.get("id")
            try:
                result = self.handle(req["method"], req.get("params") or {})
                self.io.send_result(req_id, result)
            except Exception as exc:
                self.io.send_error(req_id, exc)

    def handle(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
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
            self.sampler = instantiate_user_object(
                self.target,
                discrete_cardinalities=discrete_cardinalities,
                continuous_dims=continuous_dims,
                init_args=self.sampler_args,
                snapshot=params.get("snapshot"),
                evaluator_metadata=self.evaluator_metadata,
            )
            self.discrete_cardinalities = discrete_cardinalities
            self.continuous_dims = continuous_dims
            return {"ok": True}
        if self.sampler is None:
            raise RuntimeError("worker not initialized")
        if method == "sample_plan":
            return {"plan": self.sampler.sample_plan()}
        if method == "training_samples_remaining":
            remaining = _call_optional(self.sampler, "training_samples_remaining", None)
            return {"remaining": None if remaining is None else int(remaining)}
        if method == "produce_latent_batch":
            return self._produce_latent_batch(int(params["nr_samples"]))
        if method == "ingest_training_values":
            training_values = np.asarray(params["training_values"], dtype=np.float64).reshape((-1,))
            self.sampler.ingest_training_values(training_values)
            return {"ok": True}
        if method == "pdf":
            return self._pdf(params)
        if method == "discrete_pdf":
            return self._discrete_pdf(params)
        if method == "snapshot":
            snapshot = self.sampler.snapshot()
            if isinstance(snapshot, dict) and "save_path" not in snapshot and "save_path" in self.sampler_args:
                snapshot = {**snapshot, "save_path": self.sampler_args["save_path"]}
            return {"snapshot": snapshot}
        if method == "get_diagnostics":
            diagnostics = _call_optional(self.sampler, "get_diagnostics", {})
            return {"diagnostics": diagnostics if diagnostics is not None else {}}
        raise ValueError(f"unknown method: {method}")

    def _produce_latent_batch(self, nr_samples: int) -> dict[str, Any]:
        batch = _normalize_sample_batch(self.sampler.produce_latent_batch(nr_samples))
        discrete_dims = len(self.discrete_cardinalities or [])
        continuous_dims = int(self.continuous_dims or 0)
        xs_discrete = np.asarray(batch.xs_discrete, dtype=np.int64).reshape((nr_samples, discrete_dims))
        xs_continuous = np.asarray(batch.xs_continuous, dtype=np.float64).reshape((nr_samples, continuous_dims))
        weights = np.asarray(batch.weights, dtype=np.float64).reshape((nr_samples,))
        self._validate_sample_batch(xs_discrete, xs_continuous, weights)
        return {
            "xs_discrete_row_major": xs_discrete.reshape((nr_samples * discrete_dims,)).tolist(),
            "xs_discrete_offsets": [index * discrete_dims for index in range(nr_samples + 1)],
            "xs_continuous_row_major": xs_continuous.reshape((nr_samples * continuous_dims,)).tolist(),
            "xs_continuous_offsets": [index * continuous_dims for index in range(nr_samples + 1)],
            "weights": weights.tolist(),
        }

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
        return {"values": np.asarray(values, dtype=np.float64).reshape((nr_samples,)).tolist()}

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
            raise ValueError("produce_latent_batch returned non-finite continuous values")
        if not np.isfinite(weights).all():
            raise ValueError("produce_latent_batch returned non-finite weights")
        if (weights <= 0.0).any():
            raise ValueError("produce_latent_batch returned non-positive weights")
        for axis, cardinality in enumerate(self.discrete_cardinalities or []):
            axis_values = xs_discrete[:, axis]
            if ((axis_values < 0) | (axis_values >= cardinality)).any():
                raise ValueError(
                    f"produce_latent_batch returned discrete values outside [0, {cardinality}) on axis {axis}"
                )


def _normalize_gammaloop_result(raw: Any, nr_samples: int) -> GammaLoopBatchResult:
    if isinstance(raw, GammaLoopBatchResult):
        return raw
    if isinstance(raw, dict):
        state = raw.get("state", raw)
        training_values = raw.get("training_values")
        return GammaLoopBatchResult(state=state, training_values=training_values)
    raise TypeError(
        f"eval_gammaloop must return a GammaLoopBatchResult or dict, got {type(raw).__name__}"
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
    if not np.isfinite(array).all():
        raise ValueError("eval returned non-finite values")
    return array


def _normalize_sample_batch(batch: Any) -> SampleBatch:
    if isinstance(batch, SampleBatch):
        return batch
    if isinstance(batch, dict):
        return SampleBatch(batch["xs_discrete"], batch["xs_continuous"], batch["weights"])
    if isinstance(batch, tuple) and len(batch) == 3:
        return SampleBatch(batch[0], batch[1], batch[2])
    return SampleBatch(batch.xs_discrete, batch.xs_continuous, batch.weights)


def _call_optional(obj: Any, name: str, fallback: Any, *args: Any) -> Any:
    method = getattr(obj, name, None)
    if method is None:
        return fallback
    return method(*args)


def _read_metadata(obj: Any) -> Any:
    raw = getattr(obj, "metadata", None)
    if raw is None:
        return {}
    value = raw() if callable(raw) else raw
    return {} if value is None else value
