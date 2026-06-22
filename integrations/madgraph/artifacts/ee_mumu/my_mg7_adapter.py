from __future__ import annotations

import numpy as np


def matrix_element(momenta, parameters=None):
    """Tiny demo adapter with the same shape as a MadGraph7 callable.

    This is not a physical MadGraph result. Replace this file with an adapter
    around a MadGraph7-generated matrix element for real runs.
    """

    momenta = np.asarray(momenta, dtype=np.float64)
    return np.ones((momenta.shape[0],), dtype=np.float64)
