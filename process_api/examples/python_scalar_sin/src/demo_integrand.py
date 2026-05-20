from __future__ import annotations

import numpy as np

from gammaboard_process import Evaluator


class SinIntegrand(Evaluator):
    """Demo integrand with mixed discrete/continuous structure.

    Discrete axes:
      - spin ∈ {0, 1}
      - channel ∈ {0, 1, 2}
    Continuous axes:
      - u, v
    """

    def __init__(
        self,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        scale: float = 1.0,
        bias: float = 0.0,
        freq_u: float = 2.0,
        freq_v: float = 1.25,
        branch_weights: list[list[float]] | None = None,
        phase_offsets: list[list[float]] | None = None,
    ) -> None:
        if [int(value) for value in discrete_cardinalities] != [2, 3] or int(continuous_dims) != 2:
            raise ValueError(
                "SinIntegrand expects discrete_cardinalities=[2, 3] and continuous_dims=2"
            )
        self.scale = float(scale)
        self.bias = float(bias)
        self.freq_u = float(freq_u)
        self.freq_v = float(freq_v)
        weights = branch_weights or [[1.00, 1.15, 0.85], [1.30, 0.90, 1.05]]
        phases = phase_offsets or [[0.00, 0.35, -0.20], [0.45, -0.10, 0.25]]
        self.branch_weights = self._read_2x3(weights, "branch_weights")
        self.phase_offsets = self._read_2x3(phases, "phase_offsets")

    @staticmethod
    def _read_2x3(values: list[list[float]], name: str) -> np.ndarray:
        array = np.asarray(values, dtype=np.float64)
        if array.shape != (2, 3):
            raise ValueError(f"{name} must have shape (2, 3), got {array.shape}")
        if not np.isfinite(array).all():
            raise ValueError(f"{name} must contain only finite values")
        return array

    def eval(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> np.ndarray:
        if xs_discrete.ndim != 2 or xs_discrete.shape[1] != 2:
            raise ValueError(f"expected discrete shape (nr_samples, 2), got {xs_discrete.shape}")
        if xs_continuous.ndim != 2 or xs_continuous.shape[1] != 2:
            raise ValueError(f"expected continuous shape (nr_samples, 2), got {xs_continuous.shape}")
        if xs_discrete.shape[0] != xs_continuous.shape[0]:
            raise ValueError("discrete/continuous batch size mismatch")
        if not np.isfinite(xs_continuous).all():
            raise ValueError("continuous inputs must be finite")

        spin = xs_discrete[:, 0]
        channel = xs_discrete[:, 1]
        if ((spin < 0) | (spin >= 2)).any():
            raise ValueError("spin axis out of bounds; expected values in [0, 1]")
        if ((channel < 0) | (channel >= 3)).any():
            raise ValueError("channel axis out of bounds; expected values in [0, 2]")

        u = xs_continuous[:, 0]
        v = xs_continuous[:, 1]
        spin_f = spin.astype(np.float64)
        channel_f = channel.astype(np.float64)

        radial = (u - 0.20 * spin_f) ** 2 + (v + 0.12 * channel_f) ** 2
        phase = self.phase_offsets[spin, channel]
        branch_weight = self.branch_weights[spin, channel]
        oscillation = np.sin(
            (spin_f + 1.0) * self.freq_u * u + (channel_f + 1.0) * self.freq_v * v + phase
        )
        coupling = 1.0 + 0.15 * np.cos((spin_f - channel_f) * u * v)
        return self.bias + self.scale * branch_weight * coupling * oscillation * np.exp(-radial)
