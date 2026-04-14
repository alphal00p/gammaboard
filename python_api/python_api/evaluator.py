from __future__ import annotations

from abc import ABC, abstractmethod

from .types import ComplexOut, DiscreteBatch, RealBatch, RealOut


class ScalarBatchIntegrand(ABC):
    """Vectorized scalar integrand.

    Contract:
    - xs_discrete is an int64 array with shape (nr_samples, discrete_dims)
    - xs_continuous is a float64 array with shape (nr_samples, continuous_dims)
    - eval(xs_discrete, xs_continuous) returns float64 array with shape (nr_samples,)
    """

    discrete_dims: int
    continuous_dims: int

    @abstractmethod
    def eval(self, xs_discrete: DiscreteBatch, xs_continuous: RealBatch) -> RealOut: ...


class ComplexBatchIntegrand(ABC):
    """Vectorized complex integrand.

    Contract:
    - xs_discrete is an int64 array with shape (nr_samples, discrete_dims)
    - xs_continuous is a float64 array with shape (nr_samples, continuous_dims)
    - eval(xs_discrete, xs_continuous) returns complex128 array with shape (nr_samples,)
    """

    discrete_dims: int
    continuous_dims: int

    @abstractmethod
    def eval(self, xs_discrete: DiscreteBatch, xs_continuous: RealBatch) -> ComplexOut: ...
