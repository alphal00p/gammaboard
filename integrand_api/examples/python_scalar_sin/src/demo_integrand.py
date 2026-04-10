from __future__ import annotations

from typing import TYPE_CHECKING

import numpy as np

if TYPE_CHECKING:
    from python_api.abc import ScalarBatchIntegrand as _ScalarBatchIntegrand
else:
    class _ScalarBatchIntegrand:
        pass


class SinIntegrand(_ScalarBatchIntegrand):
    input_dim = 1

    def __init__(self, scale: float = 1.0, bias: float = 0.0) -> None:
        self.scale = float(scale)
        self.bias = float(bias)

    @classmethod
    def from_config(cls, *, input_dim: int, init_args: dict):
        if int(input_dim) != cls.input_dim:
            raise ValueError(f"expected input_dim={cls.input_dim}, got {input_dim}")
        return cls(**(init_args or {}))

    def eval(self, xs: np.ndarray) -> np.ndarray:
        x = xs[:, 0]
        return self.scale * (np.sin(x) * np.exp(-(x * x))) + self.bias
