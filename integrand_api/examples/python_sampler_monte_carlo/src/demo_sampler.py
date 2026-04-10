from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

import numpy as np

if TYPE_CHECKING:
    from python_api.abc import SamplerAggregator as _SamplerAggregator
else:
    class _SamplerAggregator:
        pass

@dataclass
class BasicMonteCarloSampler(_SamplerAggregator):
    input_dim: int
    training_target_samples: int = 0
    trained_samples: int = 0
    produced_batches: int = 0
    produced_samples: int = 0
    _rng: np.random.Generator | None = None

    def __post_init__(self) -> None:
        if self.input_dim <= 0:
            raise ValueError("input_dim must be > 0")
        if self._rng is None:
            self._rng = np.random.default_rng(0)

    @classmethod
    def from_config(cls, *, input_dim: int, init_args: dict[str, Any]):
        args = dict(init_args or {})
        seed = int(args.pop("seed", 0))
        sampler = cls(input_dim=int(input_dim), **args)
        sampler._rng = np.random.default_rng(seed)
        return sampler

    @classmethod
    def from_snapshot(cls, *, snapshot: dict[str, Any], input_dim: int, init_args: dict[str, Any]):
        args = dict(init_args or {})
        sampler = cls(
            input_dim=int(input_dim),
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

    def produce_latent_batch(self, nr_samples: int) -> np.ndarray:
        if nr_samples <= 0:
            raise ValueError("nr_samples must be > 0")
        self.produced_batches += 1
        self.produced_samples += nr_samples
        return self._rng.random((nr_samples, self.input_dim), dtype=np.float64)

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
