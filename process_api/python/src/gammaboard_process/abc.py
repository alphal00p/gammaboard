from __future__ import annotations

from abc import ABC, abstractmethod
from collections.abc import Sequence
from typing import TYPE_CHECKING, Any, Self

import numpy as np
from numpy.typing import NDArray

from .batches import Batch, GenerationStatus, SampleBatch
from .domain import Domain
from .feedback import Feedback

if TYPE_CHECKING:
    from .evaluation import EvaluationBatch, EvaluationRequest, EvaluatorDescriptor


class Evaluator(ABC):
    """Optional base class for process evaluators.

    Inheritance is not required. ``run_evaluator`` accepts any class with a
    compatible ``descriptor`` and ``evaluate`` methods; legacy ``eval`` is supported.
    Fresh workers construct evaluators as ``ClassName(domain=..., **args)``;
    domain is None during discovery.
    """

    def metadata(self) -> Any:
        """Return JSON-safe evaluator metadata passed to sampler initialization."""
        return {}

    def diagnostics(self) -> dict[str, Any]:
        return {}

    def descriptor(self) -> EvaluatorDescriptor | None:
        """Return EvaluatorDescriptor; None selects the legacy eval adapter."""
        return None

    def evaluate(
        self, xs_discrete, xs_continuous, request: EvaluationRequest
    ) -> EvaluationBatch:
        """Return EvaluationBatch with requested components and per-sample channels."""
        raise NotImplementedError("implement evaluate for a descriptor-based evaluator")

    def eval(self, xs_discrete, xs_continuous):
        """Legacy fixed-output hook; prefer descriptor() and evaluate()."""
        raise NotImplementedError("implement eval for a legacy evaluator")


class Sampler(ABC):
    """Optional base class for process samplers.

    Inheritance is not required. ``run_sampler`` accepts any class with the
    required sampler methods. Fresh workers construct samplers as
    ``ClassName(domain=..., evaluator_metadata=..., **args)``
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
    def feedback(self, feedback: Feedback) -> None:
        """Consume one complete draw; feedback.values is already masked by GammaBoard."""

    @abstractmethod
    def snapshot(self) -> Any:
        """Return non-null JSON-safe sampler state; use {} for empty state."""

    def checkpoint_committed(self, *, snapshot: Any) -> None:
        """Observe a successful database commit of this exact snapshot payload.

        Optional; the default does nothing. ``snapshot`` is the JSON-decoded
        value returned by snapshot(), without GammaBoard's checkpoint envelope.
        Delivery can be repeated or missed on process exit: make this method
        idempotent, and establish the committed state in from_snapshot() too.
        Exceptions do not undo the database commit. Older pending notifications
        are superseded when a newer checkpoint save is attempted.

        This is not permission to delete artifacts still referenced by stages,
        inference tasks or backups. Keep those according to your retention policy.
        """
        return None

    @classmethod
    @abstractmethod
    def from_snapshot(
        cls,
        *,
        snapshot: Any,
        domain: Domain,
        init_args: dict[str, Any],
        evaluator_metadata: Any = None,
    ) -> Self:
        """Restore all private state needed for pending feedback from a non-null snapshot."""
        raise NotImplementedError(
            "override from_snapshot(...) to support sampler restore"
        )

    @classmethod
    def from_stage(
        cls,
        *,
        source_state: Any,
        source_args: dict[str, Any],
        init_args: dict[str, Any],
        domain,
        evaluator_metadata: Any = None,
    ) -> Self:
        """Transfer a trained proposal into a fresh stage; no pending feedback/counters.

        Exact restoration is from_snapshot. Stratified inference assumes independent
        draws from a frozen proposal and exact normalized discrete_mass probabilities.
        No capability declaration certifies those statistical assumptions.
        """
        raise NotImplementedError("sampler does not support stage handoff")

    def pdf(
        self, xs_discrete: np.ndarray, xs_continuous: np.ndarray
    ) -> Sequence[float | None] | NDArray[Any] | None:
        """Return one normalized joint density per input point, in input order.

        Return a sequence or one-dimensional NumPy array of length N, or None
        when the whole query is unavailable. Individual entries may also be
        None for unavailable densities. Known densities must be finite and
        nonnegative; 0.0 means known zero and densities may exceed 1.0.
        """
        return None

    def discrete_mass(
        self, subspaces: list[dict[int, int]]
    ) -> Sequence[float | None] | NDArray[Any] | None:
        """Return one normalized probability mass per subspace, in input order.

        Each dictionary fixes discrete dimension indices to values. Marginalize
        over unspecified discrete dimensions and integrate continuous coordinates.
        Return a sequence or one-dimensional NumPy array of length len(subspaces),
        or None when all masses are unavailable. Entries may independently be
        None; known masses must be finite and in [0.0, 1.0]. 0.0 is known zero.
        """
        return None

    def diagnostics(self) -> dict[str, Any]:
        return {}

    def persisted_output(self) -> Any:
        """Return optional JSON task output; this is separate from recovery state."""
        return None


class Materializer(ABC):
    """Optional base class for process materializers.

    Inheritance is not required. ``run_materializer`` accepts any class with a
    compatible ``materialize_batch`` method. Fresh workers construct
    materializers as ``ClassName(domain=..., **args)``; domain is None during discovery.
    """

    @abstractmethod
    def materialize_batch(self, latent_batch: dict[str, Any]) -> Batch:
        """Return materialized sample arrays as a ``Batch``."""


class BatchTransform(ABC):
    """Optional base class for process batch transforms.

    Inheritance is not required. ``run_batch_transform`` accepts any class with
    a compatible ``transform_batch`` method. Fresh workers construct transforms
    as ``ClassName(domain=..., **args)``; domain is None during discovery.
    """

    @abstractmethod
    def transform_batch(
        self,
        xs_discrete: np.ndarray,
        xs_continuous: np.ndarray,
        weights: np.ndarray,
    ) -> Batch:
        """Return transformed sample arrays as a ``Batch``."""
