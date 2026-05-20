from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(slots=True)
class SampleBatch:
    xs_discrete: Any
    xs_continuous: Any
    weights: Any


@dataclass(slots=True)
class RaggedBatch:
    xs_discrete_row_major: list[int]
    xs_discrete_offsets: list[int]
    xs_continuous_row_major: list[float]
    xs_continuous_offsets: list[int]
    nr_samples: int

    def discrete_rows(self) -> list[list[int]]:
        return [
            self.xs_discrete_row_major[self.xs_discrete_offsets[i] : self.xs_discrete_offsets[i + 1]]
            for i in range(self.nr_samples)
        ]

    def continuous_rows(self) -> list[list[float]]:
        return [
            self.xs_continuous_row_major[self.xs_continuous_offsets[i] : self.xs_continuous_offsets[i + 1]]
            for i in range(self.nr_samples)
        ]
