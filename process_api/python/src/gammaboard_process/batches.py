from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(slots=True)
class SampleBatch:
    xs_discrete: Any
    xs_continuous: Any
    weights: Any


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
