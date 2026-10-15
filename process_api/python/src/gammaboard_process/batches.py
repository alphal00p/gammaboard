from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Any


@dataclass(slots=True)
class Batch:
    """Sample arrays returned by materializers and transforms."""

    xs_discrete: Any
    xs_continuous: Any
    weights: Any


@dataclass(slots=True)
class SampleBatch(Batch):
    """A sampler draw, with optional feedback-window information."""

    # Remaining samples in this training window before this draw; None disables feedback.
    training_remaining: int | None = None


class GenerationStatus(Enum):
    WAITING = "waiting"
    FINISHED = "finished"
