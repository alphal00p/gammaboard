from __future__ import annotations

from abc import ABC, abstractmethod
from typing import TYPE_CHECKING, Any, Self

import numpy as np
from numpy.typing import ArrayLike

from .batches import Batch, GenerationStatus, SampleBatch

if TYPE_CHECKING:
    from .gammaloop import GammaLoopBatchResult


class _EvaluatorBase(ABC):
    def metadata(self) -> Any:
        """Return JSON-safe evaluator metadata passed to sampler initialization."""
        return {}

    def diagnostics(self) -> dict[str, Any]:
        return {}


class Evaluator(_EvaluatorBase):
    """Optional base class for process evaluators.

    Inheritance is not required. ``run_evaluator`` accepts any class with a
    compatible ``eval`` method. Fresh workers construct evaluators as
    ``ClassName(discrete_cardinalities=..., continuous_dims=..., **args)``.
    """

    @abstractmethod
    def eval(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> ArrayLike | dict[str, ArrayLike]:
        """Return values with shape ``(nr_samples,)`` or ``(nr_samples, nr_components)``."""


class GammaLoopEvaluator(_EvaluatorBase):
    """Optional base for callbacks that produce GammaLoop accumulator state.

    Implement only ``eval_gammaloop``; ordinary ``eval`` is not required.
    Both evaluator interfaces use ``run_evaluator`` and the same constructor.
    """

    @abstractmethod
    def eval_gammaloop(
        self, xs_discrete: np.ndarray, xs_continuous: np.ndarray,
        weights: np.ndarray, *, require_training_values: bool,
    ) -> GammaLoopBatchResult:
        """Evaluate a batch and return a GammaLoop accumulator state.

        Override this method when the run's accumulator is configured as
        ``gammaloop``.  The returned :class:`~gammaboard_process.GammaLoopBatchResult`
        carries a JSON-serializable ``GammaLoopAccumulatorState`` dict (with at
        minimum a ``bundle`` key containing an
        :class:`~gammaboard_process.gammaloop.ObservableSnapshotBundle`) and a
        list of weighted per-sample training values when requested, otherwise None.
        Apply the supplied total weights exactly once to state and feedback.

        """


class Sampler(ABC):
    """Optional base class for process samplers.

    Inheritance is not required. ``run_sampler`` accepts any class with the
    required sampler methods. Fresh workers construct samplers as
    ``ClassName(discrete_cardinalities=..., continuous_dims=..., evaluator_metadata=..., **args)``
    including the ``evaluator_metadata`` keyword.
    """

    @abstractmethod
    def generate(self, max_samples: int) -> SampleBatch | GenerationStatus:
        """Return a SampleBatch with 1..max_samples samples, WAITING or FINISHED.

        max_samples is a positive per-draw limit supplied by queue tuning and
        capped by the remaining task budget and campaign allowance. It can change
        between calls. Set the batch's
        training_remaining to the ungenerated training window size to request
        one feedback value per sample. Evaluator splitting is runtime-owned.
        """

    @abstractmethod
    def feedback(self, values: np.ndarray) -> None:
        """Consume weighted values for exactly one draw, in generation order."""

    @abstractmethod
    def snapshot(self) -> Any:
        """Return non-null JSON-safe sampler state; use {} for empty state."""

    @classmethod
    @abstractmethod
    def from_snapshot(
        cls,
        *,
        snapshot: Any,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        init_args: dict[str, Any],
        evaluator_metadata: Any = None,
    ) -> Self:
        """Restore all private state needed for pending feedback from a non-null snapshot."""
        raise NotImplementedError("override from_snapshot(...) to support sampler restore")

    @classmethod
    def from_stage(cls, *, source_state: Any, source_args: dict[str, Any],
                   init_args: dict[str, Any], discrete_cardinalities: list[int],
                   continuous_dims: int, evaluator_metadata: Any = None) -> Self:
        """Transfer a trained proposal into a fresh stage; no pending feedback/counters.

        Exact restoration is from_snapshot. Stratified inference assumes independent
        draws from a frozen proposal and exact normalized discrete_pdf probabilities.
        No capability declaration certifies those statistical assumptions.
        """
        raise NotImplementedError("sampler does not support stage handoff")

    def pdf(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> Any:
        return None

    def discrete_pdf(self, subspaces: list[dict[int, int]]) -> Any:
        return None

    def diagnostics(self) -> dict[str, Any]:
        return {}

    def global_pdf_norm(self) -> float:
        """Return the finite positive PDF normalization used by PDF diagnostics."""
        return 1.0

    def persisted_output(self) -> Any:
        """Return optional JSON task output; this is separate from recovery state."""
        return None


class Materializer(ABC):
    """Optional base class for process materializers.

    Inheritance is not required. ``run_materializer`` accepts any class with a
    compatible ``materialize_batch`` method. Fresh workers construct
    materializers as ``ClassName(discrete_cardinalities=..., continuous_dims=..., **args)``.
    """

    @abstractmethod
    def materialize_batch(self, latent_batch: dict[str, Any]) -> Batch:
        """Return materialized sample arrays as a ``Batch``."""


class BatchTransform(ABC):
    """Optional base class for process batch transforms.

    Inheritance is not required. ``run_batch_transform`` accepts any class with
    a compatible ``transform_batch`` method. Fresh workers construct transforms
    as ``ClassName(discrete_cardinalities=..., continuous_dims=..., **args)``.
    """

    @abstractmethod
    def transform_batch(
        self,
        xs_discrete: np.ndarray,
        xs_continuous: np.ndarray,
        weights: np.ndarray,
    ) -> Batch:
        """Return transformed sample arrays as a ``Batch``."""
