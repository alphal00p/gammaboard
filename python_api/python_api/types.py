from __future__ import annotations

from typing import Any

import numpy as np
import numpy.typing as npt

DiscreteBatch = npt.NDArray[np.int64]
RealBatch = npt.NDArray[np.float64]
RealOut = npt.NDArray[np.float64]
ComplexOut = npt.NDArray[np.complex128]
SamplePlan = dict[str, Any]
Snapshot = dict[str, Any]
Diagnostics = dict[str, Any]
