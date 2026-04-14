from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

import numpy as np

if TYPE_CHECKING:
    from python_api.sampler import SamplerAggregator as _SamplerAggregator
else:
    class _SamplerAggregator:
        pass

@dataclass
class BasicMonteCarloSampler(_SamplerAggregator):
    discrete_dims: int
    continuous_dims: int
    training_target_samples: int = 0
    trained_samples: int = 0
    produced_batches: int = 0
    produced_samples: int = 0
    _rng: np.random.Generator | None = None

    def __post_init__(self) -> None:
        if self.discrete_dims != 0:
            raise ValueError("demo sampler supports only discrete_dims = 0")
        if self.continuous_dims <= 0:
            raise ValueError("continuous_dims must be > 0")
        if self._rng is None:
            self._rng = np.random.default_rng(0)

    @classmethod
    def from_config(
        cls,
        *,
        discrete_dims: int,
        continuous_dims: int,
        init_args: dict[str, Any],
    ):
        args = dict(init_args or {})
        seed = int(args.pop("seed", 0))
        sampler = cls(
            discrete_dims=int(discrete_dims),
            continuous_dims=int(continuous_dims),
            **args,
        )
        sampler._rng = np.random.default_rng(seed)
        return sampler

    @classmethod
    def from_snapshot(
        cls,
        *,
        snapshot: dict[str, Any],
        discrete_dims: int,
        continuous_dims: int,
        init_args: dict[str, Any],
    ):
        args = dict(init_args or {})
        sampler = cls(
            discrete_dims=int(discrete_dims),
            continuous_dims=int(continuous_dims),
            training_target_samples=int(snapshot.get("training_target_samples", args.get("training_target_samples", 0))),
            trained_samples=int(snapshot.get("trained_samples", 0)),
            produced_batches=int(snapshot.get("produced_batches", 0)),
            produced_samples=int(snapshot.get("produced_samples", 0)),
        )
        bitgen_state = snapshot.get("rng_state")
        if bitgen_state is None:
            seed = int(args.get("seed", 0))
            sampler._rng = np.random.default_rng(seed)
        else:
            bitgen = np.random.PCG64()
            bitgen.state = bitgen_state
            sampler._rng = np.random.Generator(bitgen)
        return sampler

    def training_samples_remaining(self) -> int | None:
        if self.training_target_samples <= 0:
            return None
        return max(0, self.training_target_samples - self.trained_samples)

    def sample_plan(self) -> dict[str, Any]:
        return {"kind": "produce", "nr_samples": 1_000_000_000}

    def produce_latent_batch(self, nr_samples: int) -> tuple[np.ndarray, np.ndarray]:
        if nr_samples <= 0:
            raise ValueError("nr_samples must be > 0")
        self.produced_batches += 1
        self.produced_samples += nr_samples
        return (
            np.zeros((nr_samples, self.discrete_dims), dtype=np.int64),
            self._rng.random((nr_samples, self.continuous_dims), dtype=np.float64),
        )

    def ingest_training_weights(self, training_weights: np.ndarray) -> None:
        self.trained_samples += int(np.asarray(training_weights).shape[0])

    def snapshot(self) -> dict[str, Any]:
        return {
            "training_target_samples": self.training_target_samples,
            "trained_samples": self.trained_samples,
            "produced_batches": self.produced_batches,
            "produced_samples": self.produced_samples,
            "rng_state": self._rng.bit_generator.state,
        }

    def get_diagnostics(self) -> dict[str, Any]:
        return {
            "produced_batches": self.produced_batches,
            "produced_samples": self.produced_samples,
            "trained_samples": self.trained_samples,
            "training_samples_remaining": self.training_samples_remaining(),
        }

    def pdf(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> np.ndarray:
        xs_discrete = np.asarray(xs_discrete, dtype=np.int64)
        xs_continuous = np.asarray(xs_continuous, dtype=np.float64)
        if xs_discrete.ndim != 2 or xs_discrete.shape[1] != self.discrete_dims:
            raise ValueError(
                f"expected discrete xs with shape (nr_samples, {self.discrete_dims}), got {xs_discrete.shape}"
            )
        if xs_continuous.ndim != 2 or xs_continuous.shape[1] != self.continuous_dims:
            raise ValueError(
                f"expected continuous xs with shape (nr_samples, {self.continuous_dims}), got {xs_continuous.shape}"
            )
        return np.ones((xs_continuous.shape[0],), dtype=np.float64)
