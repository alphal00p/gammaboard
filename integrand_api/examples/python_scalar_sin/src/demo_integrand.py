from __future__ import annotations

import numpy as np


class SinIntegrand:
    input_dim = 1

    def eval(self, xs: np.ndarray) -> np.ndarray:
        x = xs[:, 0]
        return np.sin(x) * np.exp(-(x * x))
