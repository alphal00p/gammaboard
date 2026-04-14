from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Any

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
    def sample_plan(self) -> SamplePlan:
        """Return a JSON-serializable description of what the sampler will produce.

        Example:
            {
                "kind": "monte_carlo",
                "discrete_dims": 2,
                "continuous_dims": 1,
                "batch_hint": 256,         # advisory preferred batch size
                "meta": {"seed": 42}     # sampler-specific params
            }

        Fields are advisory and primarily for frontend/display use. Callers should not
        assume strict validation beyond documented keys.
        """
        raise NotImplementedError

    @abstractmethod
    def training_samples_remaining(self) -> int | None:
        """Return remaining training samples.

        None means there is no fixed training phase or the remaining count is
        unknown/unbounded.
        """
        raise NotImplementedError

    @abstractmethod
    def produce_latent_batch(self, nr_samples: int) -> tuple[DiscreteBatch, RealBatch]:
        """Produce a latent batch.

        Returns two arrays (discrete, continuous) where the first dimension == nr_samples.
        The caller is responsible for requesting an appropriate nr_samples and for
        matching batch sizes when ingesting training weights.
        """
        raise NotImplementedError

    @abstractmethod
    def ingest_training_weights(self, training_weights: RealOut) -> None:
        """Provide per-sample training weights for the most recently produced batch.

        Must be called after produce_latent_batch with a matching length. Implementations
        may assume this ordering and may update internal training progress/state.
        """
        raise NotImplementedError

    @abstractmethod
    def snapshot(self) -> Snapshot:
        """Return a JSON-serializable snapshot suitable for persistence.

        The snapshot must contain enough information for from_snapshot to restore
        an equivalent sampler instance.
        """
        raise NotImplementedError

    @classmethod
    @abstractmethod
    def from_snapshot(
        cls,
        *,
        snapshot: Snapshot,
        discrete_dims: int,
        continuous_dims: int,
        init_args: dict | None = None,
    ) -> "SamplerAggregator":
        """Factory to restore an instance from a snapshot produced by snapshot().

        Args:
            snapshot: JSON-serializable state from snapshot().
            discrete_dims, continuous_dims: expected dimensionality (may be used for validation).
            init_args: optional initialization args that were passed via from_config.

        Implementations should accept only JSON-serializable snapshots and may raise
        on incompatible or corrupted snapshots.
        """
        raise NotImplementedError

    @classmethod
    @abstractmethod
    def from_config(
        cls,
        *,
        discrete_dims: int,
        continuous_dims: int,
        init_args: dict | None = None,
    ) -> "SamplerAggregator":
        """Factory to construct a fresh sampler from configuration.

        This mirrors example samplers' from_config signature. init_args is the
        framework-provided module init dict and may be empty.
        """
        raise NotImplementedError

    def get_diagnostics(self) -> Diagnostics:
        """Optional runtime diagnostics. Empty dict means no diagnostics available."""
        return {}

    def pdf(self, xs_discrete: DiscreteBatch, xs_continuous: RealBatch) -> RealOut | None:
        """Return per-sample PDF values if supported.

        Return a float64 array with shape (nr_samples,) or None to signal that
        the sampler does not support/doesn't provide a PDF for the given batch.
        """
        return None
