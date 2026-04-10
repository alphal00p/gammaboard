from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Any

import numpy as np
import numpy.typing as npt

RealBatch = npt.NDArray[np.float64]
RealOut = npt.NDArray[np.float64]
ComplexOut = npt.NDArray[np.complex128]
SamplePlan = dict[str, Any]
Snapshot = dict[str, Any]
Diagnostics = dict[str, Any]


class ScalarBatchIntegrand(ABC):
    """Vectorized scalar integrand.

    Contract:
    - xs is a float64 array with shape (nr_samples, input_dim)
    - eval(xs) returns float64 array with shape (nr_samples,)
    """

    input_dim: int

    @abstractmethod
    def eval(self, xs: RealBatch) -> RealOut: ...


class ComplexBatchIntegrand(ABC):
    """Vectorized complex integrand.

    Contract:
    - xs is a float64 array with shape (nr_samples, input_dim)
    - eval(xs) returns complex128 array with shape (nr_samples,)
    """

    input_dim: int

    @abstractmethod
    def eval(self, xs: RealBatch) -> ComplexOut: ...


class SamplerAggregator(ABC):
    """Vectorized homogeneous sampler-aggregator.

    Contract:
    - produce_latent_batch(nr_samples) returns float64 array with shape
      (nr_samples, input_dim)
    - ingest_training_weights(training_weights) receives float64 array with shape
      (nr_samples,)
    - sample_plan() returns a JSON-serializable dict
    - snapshot() returns a JSON-serializable dict
    """

    input_dim: int

    @abstractmethod
    def sample_plan(self) -> SamplePlan: ...

    @abstractmethod
    def training_samples_remaining(self) -> int | None: ...

    @abstractmethod
    def produce_latent_batch(self, nr_samples: int) -> RealBatch: ...

    @abstractmethod
    def ingest_training_weights(self, training_weights: RealOut) -> None: ...

    @abstractmethod
    def snapshot(self) -> Snapshot: ...

    def get_diagnostics(self) -> Diagnostics:
        return {}

    def pdf(self, xs: RealBatch) -> RealOut:
        raise NotImplementedError("pdf(...) not implemented for this sampler")
