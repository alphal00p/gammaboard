from __future__ import annotations

from abc import ABC, abstractmethod
from typing import Any

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

    @classmethod
    @abstractmethod
    def from_config(cls, *, discrete_dims: int, continuous_dims: int, init_args: dict[str, Any] | None):
        """Optional factory to construct a fresh integrand from configuration.

        Mirrors example integrand signatures. Implementations may validate dims and
        return a configured instance.
        """
        raise NotImplementedError

    @abstractmethod
    def eval(self, xs_discrete: DiscreteBatch, xs_continuous: RealBatch) -> RealOut:
        """Must be purely functional and vectorized: return array with length == xs_discrete.shape[0].

        Avoid side-effects; callers may invoke concurrently.
        """
        raise NotImplementedError


class ComplexBatchIntegrand(ABC):
    """Vectorized complex integrand.

    Contract:
    - xs_discrete is an int64 array with shape (nr_samples, discrete_dims)
    - xs_continuous is a float64 array with shape (nr_samples, continuous_dims)
    - eval(xs_discrete, xs_continuous) returns complex128 array with shape (nr_samples,)
    """

    discrete_dims: int
    continuous_dims: int

    @classmethod
    @abstractmethod
    def from_config(cls, *, discrete_dims: int, continuous_dims: int, init_args: dict[str, Any] | None):
        """Optional factory to construct a fresh integrand from configuration.

        Mirrors example integrand signatures. Implementations may validate dims and
        return a configured instance.
        """
        raise NotImplementedError

    @abstractmethod
    def eval(self, xs_discrete: DiscreteBatch, xs_continuous: RealBatch) -> ComplexOut:
        """Must be purely functional and vectorized: return array with length == xs_discrete.shape[0].

        Avoid side-effects; callers may invoke concurrently.
        """
        raise NotImplementedError
