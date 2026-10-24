"""Structural domains shared by all process roles; probabilities live in samplers."""

from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from itertools import product


class DiscreteAxisRole(StrEnum):
    INTEGRAND = "integrand"
    STRATUM = "stratum"


class Domain:
    def to_dict(self) -> dict:
        if isinstance(self, RectangularDomain):
            return {
                "rectangular": {
                    "discrete_cardinalities": list(self.discrete_cardinalities),
                    "continuous_dims": self.continuous_dims,
                    "strata_dims": self.strata_dims,
                }
            }
        return {
            "discrete": {
                "axis_label": self.axis_label,
                "axis_role": self.axis_role.value,
                "branches": [
                    {"index": b.index, "domain": b.domain.to_dict()}
                    for b in self.branches
                ],
            }
        }

    @staticmethod
    def from_dict(payload: dict) -> Domain:
        if not isinstance(payload, dict) or len(payload) != 1:
            raise ValueError("domain must contain exactly one variant")
        if "rectangular" in payload:
            spec = payload["rectangular"]
            result = RectangularDomain(
                tuple(spec["discrete_cardinalities"]),
                spec["continuous_dims"],
                spec.get("strata_dims", 0),
            )
        elif "discrete" in payload:
            spec = payload["discrete"]
            result = DiscreteDomain(
                tuple(
                    DomainBranch(b["index"], Domain.from_dict(b["domain"]))
                    for b in spec["branches"]
                ),
                DiscreteAxisRole(spec.get("axis_role", "integrand")),
                spec.get("axis_label"),
            )
        else:
            raise ValueError("unknown domain variant")
        result.validate()
        return result

    def validate(self, integrand_seen: bool = False) -> None:
        if isinstance(self, RectangularDomain):
            numbers = (
                *self.discrete_cardinalities,
                self.continuous_dims,
                self.strata_dims,
            )
            if any(type(n) is not int or n < 0 for n in numbers):
                raise ValueError("domain dimensions must be nonnegative integers")
            if any(
                n == 0 for n in self.discrete_cardinalities
            ) or self.strata_dims > len(self.discrete_cardinalities):
                raise ValueError("invalid rectangular cardinalities or strata_dims")
            if integrand_seen and self.strata_dims:
                raise ValueError("stratum dimensions must be left-bound")
        else:
            if not self.branches or any(
                type(b.index) is not int or b.index < 0 for b in self.branches
            ):
                raise ValueError("invalid discrete branches")
            if any(
                a.index >= b.index for a, b in zip(self.branches, self.branches[1:])
            ):
                raise ValueError("branch labels must be strictly increasing")
            stratum = self.axis_role == DiscreteAxisRole.STRATUM
            if integrand_seen and stratum:
                raise ValueError("stratum dimensions must be left-bound")
            for branch in self.branches:
                branch.domain.validate(integrand_seen or not stratum)

    def stratum_prefixes(self) -> tuple[tuple[int, ...], ...]:
        self.validate()
        if isinstance(self, RectangularDomain):
            return tuple(
                product(
                    *(range(n) for n in self.discrete_cardinalities[: self.strata_dims])
                )
            )
        if self.axis_role == DiscreteAxisRole.INTEGRAND:
            return ((),)
        return tuple(
            (b.index, *tail)
            for b in self.branches
            for tail in b.domain.stratum_prefixes()
        )

    def require_rectangular_shape(self) -> tuple[list[int], int]:
        """A convenience for algorithms that require homogeneous, contiguous rows."""
        if isinstance(self, RectangularDomain):
            return list(self.discrete_cardinalities), self.continuous_dims
        if [b.index for b in self.branches] != list(range(len(self.branches))):
            raise ValueError("rectangular shape requires contiguous branch labels")
        shapes = [b.domain.require_rectangular_shape() for b in self.branches]
        if any(shape != shapes[0] for shape in shapes):
            raise ValueError("rectangular shape does not support inhomogeneous domains")
        return [len(self.branches), *shapes[0][0]], shapes[0][1]

    def fixed_widths(self) -> tuple[int | None, int | None]:
        if isinstance(self, RectangularDomain):
            return len(self.discrete_cardinalities), self.continuous_dims
        widths = [b.domain.fixed_widths() for b in self.branches]
        d, c = widths[0]
        return (
            d + 1 if d is not None and all(w[0] == d for w in widths) else None,
            c if all(w[1] == c for w in widths) else None,
        )

    def validate_point(self, discrete, continuous) -> None:
        domain, depth = self, 0
        while isinstance(domain, DiscreteDomain):
            if depth >= len(discrete):
                raise ValueError("incomplete discrete path")
            branch = next(
                (b for b in domain.branches if b.index == discrete[depth]), None
            )
            if branch is None:
                raise ValueError("unknown discrete branch")
            domain, depth = branch.domain, depth + 1
        if (
            len(discrete) - depth != len(domain.discrete_cardinalities)
            or len(continuous) != domain.continuous_dims
        ):
            raise ValueError("point dimensions do not match domain")
        if any(
            x < 0 or x >= n
            for x, n in zip(discrete[depth:], domain.discrete_cardinalities)
        ):
            raise ValueError("discrete coordinate out of bounds")


@dataclass(frozen=True)
class RectangularDomain(Domain):
    discrete_cardinalities: tuple[int, ...]
    continuous_dims: int
    strata_dims: int = 0


@dataclass(frozen=True)
class DomainBranch:
    index: int
    domain: Domain


@dataclass(frozen=True)
class DiscreteDomain(Domain):
    branches: tuple[DomainBranch, ...]
    axis_role: DiscreteAxisRole = DiscreteAxisRole.INTEGRAND
    axis_label: str | None = None
