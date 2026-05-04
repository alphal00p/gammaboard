from __future__ import annotations

from typing import Any, Self

import numpy as np
import numpy.typing as npt


class ScalarBatchIntegrand:
    """Vectorized scalar integrand.

    Implementations are loaded by the worker through duck typing; inheriting from
    this class is optional.
    """

    discrete_cardinalities: list[int]
    continuous_dims: int

    @classmethod
    def from_config(
        cls,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        init_args: dict[str, Any] | None,
    ) -> Self:
        return cls(**(init_args or {}))

    def eval(
        self,
        xs_discrete: npt.NDArray[np.int64],
        xs_continuous: npt.NDArray[np.float64],
    ) -> npt.NDArray[np.float64]:
        raise NotImplementedError


class ComplexBatchIntegrand:
    """Vectorized complex integrand."""

    discrete_cardinalities: list[int]
    continuous_dims: int

    @classmethod
    def from_config(
        cls,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        init_args: dict[str, Any] | None,
    ) -> Self:
        return cls(**(init_args or {}))

    def eval(
        self,
        xs_discrete: npt.NDArray[np.int64],
        xs_continuous: npt.NDArray[np.float64],
    ) -> npt.NDArray[np.complex128]:
        raise NotImplementedError
