from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Self

import numpy as np
import numpy.typing as npt


@dataclass(frozen=True)
class SampleBatch:
    xs_discrete: npt.NDArray[np.int64]
    xs_continuous: npt.NDArray[np.float64]
    weights: npt.NDArray[np.float64]


class SamplerAggregator:
    """Vectorized homogeneous sampler-aggregator.

    Implementations are loaded by the worker through duck typing; inheriting from
    this class is optional. Arrays are shaped as:
    - xs_discrete: (nr_samples, len(discrete_cardinalities))
    - xs_continuous: (nr_samples, continuous_dims)
    - weights: (nr_samples,)
    """

    discrete_cardinalities: list[int]
    continuous_dims: int

    @classmethod
    def from_config(
        cls,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        init_args: dict[str, Any] | None = None,
    ) -> Self:
        return cls(**(init_args or {}))

    @classmethod
    def from_snapshot(
        cls,
        *,
        snapshot: dict[str, Any],
        discrete_cardinalities: list[int],
        continuous_dims: int,
        init_args: dict[str, Any] | None = None,
    ) -> Self:
        raise NotImplementedError

    def sample_plan(self) -> dict[str, Any]:
        return {"kind": "produce", "nr_samples": 2**63 - 1}

    def training_samples_remaining(self) -> int | None:
        return None

    def produce_latent_batch(self, nr_samples: int) -> SampleBatch:
        raise NotImplementedError

    def ingest_training_values(
        self, training_values: npt.NDArray[np.float64]
    ) -> None:
        raise NotImplementedError

    def snapshot(self) -> dict[str, Any]:
        return {}

    def get_diagnostics(self) -> dict[str, Any]:
        return {}

    def pdf(
        self,
        xs_discrete: npt.NDArray[np.int64],
        xs_continuous: npt.NDArray[np.float64],
    ) -> npt.NDArray[np.float64] | None:
        return None
