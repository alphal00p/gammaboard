from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Any


@dataclass(slots=True)
class SampleBatch:
    xs_discrete: Any
    xs_continuous: Any
    weights: Any

    # Remaining samples in this training window before this draw; None disables feedback.
    training_remaining: int | None = None


class GenerationStatus(Enum):
    WAITING = "waiting"
    FINISHED = "finished"


@dataclass(slots=True)
class MaterializedBatch:
    xs_discrete: Any
    xs_continuous: Any
    weights: Any


@dataclass(slots=True)
class TransformedBatch:
    xs_discrete: Any
    xs_continuous: Any
    weights: Any
