from __future__ import annotations

import numpy as np
from gammaboard_process import Evaluator, run_evaluator

from .state import MadSpaceState


class MadGraphEvaluator(Evaluator):
    def __init__(
        self,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        state_path: str,
        madgraph_root: str | None = None,
        subprocess_index: int = 0,
        flavor_index: int = 0,
        output: list[str] | None = None,
    ) -> None:
        if discrete_cardinalities:
            raise ValueError("MadGraph evaluator does not use discrete dimensions")
        self.state = MadSpaceState.load(
            state_path=state_path,
            madgraph_root=madgraph_root,
            subprocess_index=subprocess_index,
            flavor_index=flavor_index,
        )
        if int(continuous_dims) != self.state.random_dim:
            raise ValueError(
                f"continuous_dims={continuous_dims} does not match "
                f"{self.state.random_dim} random dimensions required by MadSpace"
            )
        self.output = output or ["weight"]
        allowed = {"weight"}
        unknown = [name for name in self.output if name not in allowed]
        if unknown:
            raise ValueError(f"unsupported MadGraph output component(s): {unknown!r}")

    @property
    def metadata(self) -> dict:
        return self.state.metadata

    def eval(
        self, xs_discrete: np.ndarray, xs_continuous: np.ndarray
    ) -> dict[str, np.ndarray]:
        if xs_discrete.shape[1] != 0:
            raise ValueError("MadGraph evaluator expects no discrete coordinates")
        weight = self.state.evaluate(xs_continuous)
        return {name: weight for name in self.output}


def main() -> None:
    run_evaluator(MadGraphEvaluator)


if __name__ == "__main__":
    main()
