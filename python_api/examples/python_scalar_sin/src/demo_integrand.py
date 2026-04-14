from __future__ import annotations

from typing import TYPE_CHECKING

import numpy as np

if TYPE_CHECKING:
    from python_api.evaluator import ScalarBatchIntegrand as _ScalarBatchIntegrand
else:
    class _ScalarBatchIntegrand:
        pass


class SinIntegrand(_ScalarBatchIntegrand):
    discrete_dims = 0
    continuous_dims = 1

    def __init__(self, scale: float = 1.0, bias: float = 0.0) -> None:
        self.scale = float(scale)
        self.bias = float(bias)

    @classmethod
    def from_config(cls, *, discrete_dims: int, continuous_dims: int, init_args: dict):
        if int(discrete_dims) != cls.discrete_dims:
            raise ValueError(f"expected discrete_dims={cls.discrete_dims}, got {discrete_dims}")
        if int(continuous_dims) != cls.continuous_dims:
            raise ValueError(
                f"expected continuous_dims={cls.continuous_dims}, got {continuous_dims}"
            )
        return cls(**(init_args or {}))

    def eval(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> np.ndarray:
        if xs_discrete.shape[1] != 0:
            raise ValueError(f"expected no discrete dimensions, got {xs_discrete.shape}")
        x = xs_continuous[:, 0]
        return self.scale * (np.sin(x) * np.exp(-(x * x))) + self.bias
