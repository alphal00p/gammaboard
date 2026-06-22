from __future__ import annotations

from typing import Any

import numpy as np
from gammaboard_process import Evaluator, run_evaluator

from .artifacts import MatrixElementBackend, load_matrix_element_backend
from .phase_space import TwoBodyPhaseSpace, build_phase_space


class MadGraphEvaluator(Evaluator):
    def __init__(
        self,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        ecm: float,
        matrix_element: dict[str, Any],
        phase_space: dict[str, Any] | None = None,
        output: list[str] | None = None,
    ) -> None:
        if discrete_cardinalities:
            raise ValueError("the first MadGraph evaluator version does not use discrete dimensions")
        self.phase_space: TwoBodyPhaseSpace = build_phase_space(phase_space or {}, ecm=float(ecm))
        if int(continuous_dims) != self.phase_space.dims:
            raise ValueError(
                f"continuous_dims={continuous_dims} does not match "
                f"{self.phase_space.dims} required by phase_space.kind='two_body'"
            )
        self.backend: MatrixElementBackend = load_matrix_element_backend(matrix_element)
        self.output = output or ["value"]
        allowed = {"value", "matrix_element", "phase_space_weight"}
        unknown = [name for name in self.output if name not in allowed]
        if unknown:
            raise ValueError(f"unsupported MadGraph output component(s): {unknown}")

    def eval(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> dict[str, np.ndarray]:
        if xs_discrete.shape[1] != 0:
            raise ValueError("the first MadGraph evaluator version expects no discrete coordinates")
        mapped = self.phase_space.map(xs_continuous)
        matrix_element = self.backend.evaluate(mapped.momenta)
        value = matrix_element * mapped.weight
        columns = {
            "value": value,
            "matrix_element": matrix_element,
            "phase_space_weight": mapped.weight,
        }
        return {name: columns[name] for name in self.output}


def main() -> None:
    run_evaluator(MadGraphEvaluator)


if __name__ == "__main__":
    main()
