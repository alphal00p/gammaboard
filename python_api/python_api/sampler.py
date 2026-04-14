from __future__ import annotations

from abc import ABC, abstractmethod

from .types import Diagnostics, DiscreteBatch, RealBatch, RealOut, SamplePlan, Snapshot


class SamplerAggregator(ABC):
    """Vectorized homogeneous sampler-aggregator.

    Contract:
    - produce_latent_batch(nr_samples) returns a pair:
      discrete int64 array with shape (nr_samples, discrete_dims) and
      continuous float64 array with shape (nr_samples, continuous_dims)
    - ingest_training_weights(training_weights) receives float64 array with shape
      (nr_samples,)
    - pdf(xs_discrete, xs_continuous) receives the same batch-shaped arrays and
      returns float64 array with shape (nr_samples,) or None
    - sample_plan() returns a JSON-serializable dict
    - snapshot() returns a JSON-serializable dict
    """

    discrete_dims: int
    continuous_dims: int

    @abstractmethod
    def sample_plan(self) -> SamplePlan: ...

    @abstractmethod
    def training_samples_remaining(self) -> int | None: ...

    @abstractmethod
    def produce_latent_batch(self, nr_samples: int) -> tuple[DiscreteBatch, RealBatch]: ...

    @abstractmethod
    def ingest_training_weights(self, training_weights: RealOut) -> None: ...

    @abstractmethod
    def snapshot(self) -> Snapshot: ...

    def get_diagnostics(self) -> Diagnostics:
        return {}

    def pdf(self, xs_discrete: DiscreteBatch, xs_continuous: RealBatch) -> RealOut | None:
        return None
