from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Any

import numpy as np


@dataclass
class PhaseSpacePoint:
    momenta: np.ndarray
    weight: np.ndarray


class TwoBodyPhaseSpace:
    dims = 2

    def __init__(
        self,
        *,
        ecm: float,
        final_state_masses: list[float] | tuple[float, float] | None = None,
        include_flux: bool = True,
    ) -> None:
        self.ecm = float(ecm)
        self.s = self.ecm * self.ecm
        masses = list(final_state_masses or [0.0, 0.0])
        if len(masses) != 2:
            raise ValueError("two_body phase space requires exactly two final_state_masses")
        self.m1 = float(masses[0])
        self.m2 = float(masses[1])
        self.include_flux = bool(include_flux)
        if self.ecm <= 0.0:
            raise ValueError("ecm must be > 0")
        if self.m1 < 0.0 or self.m2 < 0.0:
            raise ValueError("final-state masses must be >= 0")
        if self.ecm < self.m1 + self.m2:
            raise ValueError("ecm is below the two-body mass threshold")

    def map(self, xs: np.ndarray) -> PhaseSpacePoint:
        xs = np.asarray(xs, dtype=np.float64)
        if xs.ndim != 2 or xs.shape[1] != self.dims:
            raise ValueError(f"two_body phase space expects xs shape (n, {self.dims})")
        if not np.isfinite(xs).all() or (xs < 0.0).any() or (xs > 1.0).any():
            raise ValueError("phase-space coordinates must be finite values in [0, 1]")

        nr_samples = xs.shape[0]
        cos_theta = 2.0 * xs[:, 0] - 1.0
        sin_theta = np.sqrt(np.maximum(0.0, 1.0 - cos_theta * cos_theta))
        phi = 2.0 * math.pi * xs[:, 1]

        momentum = math.sqrt(
            max(0.0, _kallen(self.s, self.m1 * self.m1, self.m2 * self.m2))
        ) / (2.0 * self.ecm)
        e1 = math.sqrt(momentum * momentum + self.m1 * self.m1)
        e2 = math.sqrt(momentum * momentum + self.m2 * self.m2)

        px = momentum * sin_theta * np.cos(phi)
        py = momentum * sin_theta * np.sin(phi)
        pz = momentum * cos_theta

        momenta = np.zeros((nr_samples, 4, 4), dtype=np.float64)
        beam_energy = 0.5 * self.ecm
        momenta[:, 0, :] = np.array([beam_energy, 0.0, 0.0, beam_energy])
        momenta[:, 1, :] = np.array([beam_energy, 0.0, 0.0, -beam_energy])
        momenta[:, 2, 0] = e1
        momenta[:, 2, 1] = px
        momenta[:, 2, 2] = py
        momenta[:, 2, 3] = pz
        momenta[:, 3, 0] = e2
        momenta[:, 3, 1] = -px
        momenta[:, 3, 2] = -py
        momenta[:, 3, 3] = -pz

        # dPhi_2 = |p|/(16*pi^2*sqrt(s)) dOmega. The unit-square map has
        # dOmega = 4*pi du dv. The optional massless-initial flux is 1/(2s).
        weight = np.full(nr_samples, momentum / (4.0 * math.pi * self.ecm), dtype=np.float64)
        if self.include_flux:
            weight /= 2.0 * self.s
        return PhaseSpacePoint(momenta=momenta, weight=weight)


def build_phase_space(config: dict[str, Any], *, ecm: float) -> TwoBodyPhaseSpace:
    kind = str(config.get("kind", "two_body"))
    if kind != "two_body":
        raise ValueError("only phase_space.kind = 'two_body' is implemented initially")
    return TwoBodyPhaseSpace(
        ecm=ecm,
        final_state_masses=config.get("final_state_masses"),
        include_flux=bool(config.get("include_flux", True)),
    )


def _kallen(x: float, y: float, z: float) -> float:
    return x * x + y * y + z * z - 2.0 * (x * y + x * z + y * z)
