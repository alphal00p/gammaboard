from __future__ import annotations

from abc import ABC, abstractmethod

import numpy as np
import numpy.typing as npt

RealBatch = npt.NDArray[np.float64]
RealOut = npt.NDArray[np.float64]
ComplexOut = npt.NDArray[np.complex128]


class ScalarBatchIntegrand(ABC):
    """Vectorized scalar integrand.

    Contract:
    - xs is a float64 array with shape (nr_samples, input_dim)
    - eval(xs) returns float64 array with shape (nr_samples,)
    """

    input_dim: int

    @abstractmethod
    def eval(self, xs: RealBatch) -> RealOut: ...


class ComplexBatchIntegrand(ABC):
    """Vectorized complex integrand.

    Contract:
    - xs is a float64 array with shape (nr_samples, input_dim)
    - eval(xs) returns complex128 array with shape (nr_samples,)
    """

    input_dim: int

    @abstractmethod
    def eval(self, xs: RealBatch) -> ComplexOut: ...


class SamplerAggregator(ABC):
    pass  # TODO
