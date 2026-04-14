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

    # JSON-serializable description of what the sampler will produce; safe to persist.
    # Example:
    #     {
    #         "kind": "monte_carlo",
    #         "discrete_dims": 2,
    #         "continuous_dims": 1,
    #         "batch_hint": 256,         # advisory preferred batch size
    #         "meta": {"seed": 42}     # sampler-specific params
    #     }
    # Fields are advisory; callers should not assume strict validation beyond the documented keys.
    # these are used for the frontend display

    @abstractmethod
    def training_samples_remaining(self) -> int | None: ...

    # Returns remaining training samples; None means "no fixed training phase" or unknown/unbounded.

    @abstractmethod
    def produce_latent_batch(
        self, nr_samples: int
    ) -> tuple[DiscreteBatch, RealBatch]: ...

    # Returns two arrays with first dimension == nr_samples. Caller is responsible for matching batch sizes.

    @abstractmethod
    def ingest_training_weights(self, training_weights: RealOut) -> None: ...

    # Per-sample weights for the most recently produced batch. Must be called after produce_latent_batch
    # with a matching length; implementations may assume this ordering.

    @abstractmethod
    def snapshot(self) -> Snapshot: ...

    # JSON-serializable state sufficient to restore the sampler.

    @classmethod
    @abstractmethod
    def from_snapshot(cls, snapshot: Snapshot) -> "SamplerAggregator": ...
    # Factory to restore an instance from a snapshot produced by snapshot().
    # Implementations should accept only JSON-serializable snapshots and may raise
    # an exception for incompatible or corrupted snapshots.

    def get_diagnostics(self) -> Diagnostics:
        return {}

    # Optional runtime diagnostics; empty dict means no diagnostics available.

    def pdf(
        self, xs_discrete: DiscreteBatch, xs_continuous: RealBatch
    ) -> RealOut | None:
        return None

    # Return per-sample pdf values if supported. Returning None signals that PDF is unsupported or not available.
