"""GammaBoard-owned masking, with optional richer training information."""

from dataclasses import dataclass

import numpy as np


@dataclass
class StratumAssignments:
    prefixes: tuple[tuple[int, ...], ...]
    requested_labels: np.ndarray
    actual_labels: np.ndarray

    @property
    def requested(self) -> list[tuple[int, ...]]:
        return [self.prefixes[i] for i in self.requested_labels]

    @property
    def actual(self) -> list[tuple[int, ...]]:
        return [self.prefixes[i] for i in self.actual_labels]


@dataclass
class Feedback:
    unmasked_values: np.ndarray
    strata: StratumAssignments | None = None

    def __post_init__(self):
        self.unmasked_values = np.asarray(self.unmasked_values, dtype=np.float64)
        if (
            self.unmasked_values.ndim != 1
            or not np.isfinite(self.unmasked_values).all()
        ):
            raise ValueError("feedback needs finite one-dimensional unmasked values")
        if self.strata is not None:
            s = self.strata
            if (
                not s.prefixes
                or any(
                    not p or any(type(x) is not int or x < 0 for x in p)
                    for p in s.prefixes
                )
                or any(
                    a >= b or b[: len(a)] == a
                    for a, b in zip(s.prefixes, s.prefixes[1:])
                )
            ):
                raise ValueError("invalid feedback stratum prefixes")
            for labels in (s.requested_labels, s.actual_labels):
                if (
                    labels.shape != self.unmasked_values.shape
                    or labels.dtype.kind not in "iu"
                    or (labels < 0).any()
                    or (labels >= len(s.prefixes)).any()
                ):
                    raise ValueError("invalid feedback stratum assignments")

    @property
    def values(self) -> np.ndarray:
        if self.strata is None:
            return self.unmasked_values
        return np.where(
            self.strata.requested_labels == self.strata.actual_labels,
            self.unmasked_values,
            0.0,
        )
