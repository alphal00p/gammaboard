from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Any

import numpy as np


class Evaluator(ABC):
    """Optional base class for process evaluators.

    Inheritance is not required. ``run_evaluator`` accepts any class with a
    compatible ``eval`` method. Fresh workers construct evaluators as
    ``ClassName(discrete_cardinalities=..., continuous_dims=..., **args)``.
    """

    @abstractmethod
    def eval(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> Any:
        """Return values with shape ``(nr_samples,)`` or ``(nr_samples, nr_components)``."""

    def metadata(self) -> Any:
        """Return JSON-safe evaluator metadata passed to sampler initialization."""
        return {}


class Sampler(ABC):
    """Optional base class for process samplers.

    Inheritance is not required. ``run_sampler`` accepts any class with the
    required sampler methods. Fresh workers construct samplers as
    ``ClassName(discrete_cardinalities=..., continuous_dims=..., evaluator_metadata=..., **args)``
    when the class accepts the ``evaluator_metadata`` keyword.
    """

    @abstractmethod
    def sample_plan(self) -> dict[str, Any]:
        """Return a sampler plan, e.g. ``{"kind": "produce", "nr_samples": 1024}``."""

    @abstractmethod
    def produce_latent_batch(self, nr_samples: int) -> Any:
        """Return a ``SampleBatch`` or compatible object."""

    @abstractmethod
    def ingest_training_values(self, training_values: np.ndarray) -> None:
        """Ingest one scalar training value per produced training sample."""

    @abstractmethod
    def snapshot(self) -> Any:
        """Return JSON-safe sampler state."""

    @classmethod
    def from_snapshot(
        cls,
        *,
        snapshot: Any,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        init_args: dict[str, Any],
        evaluator_metadata: Any = None,
    ) -> Any:
        """Optional constructor used when restoring a persisted sampler snapshot."""
        raise NotImplementedError("override from_snapshot(...) to support sampler restore")

    def training_samples_remaining(self) -> int | None:
        return None

    def pdf(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> Any:
        return None

    def discrete_pdf(self, subspaces: list[dict[int, int]]) -> Any:
        return None

    def get_diagnostics(self) -> dict[str, Any]:
        return {}
