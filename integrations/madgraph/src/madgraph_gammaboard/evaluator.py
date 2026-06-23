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
        phase_space: str = "flat",
        channel_index: int = 0,
        device: str = "cppnone",
        thread_count: int = -1,
        output: list[str] | None = None,
    ) -> None:
        if discrete_cardinalities:
            raise ValueError("MadGraph evaluator does not use discrete dimensions")
        self.state = MadSpaceState.load(
            state_path=state_path,
            madgraph_root=madgraph_root,
            subprocess_index=subprocess_index,
            phase_space=phase_space,
            channel_index=channel_index,
            device=device,
            thread_count=thread_count,
        )
        if int(continuous_dims) != self.state.random_dim:
            raise ValueError(
                f"continuous_dims={continuous_dims} does not match "
                f"{self.state.random_dim} random dimensions required by the MadSpace integrand"
            )
        self.output = output or ["weight"]
        allowed = {"weight"}
        unknown = [name for name in self.output if name not in allowed]
        if unknown:
            raise ValueError(f"unsupported MadGraph output component(s): {unknown!r}")

    def eval(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> dict[str, np.ndarray]:
        if xs_discrete.shape[1] != 0:
            raise ValueError("MadGraph evaluator expects no discrete coordinates")
        weight = self.state.evaluate(xs_continuous)
        return {name: weight for name in self.output}


def main() -> None:
    run_evaluator(MadGraphEvaluator)


if __name__ == "__main__":
    main()
