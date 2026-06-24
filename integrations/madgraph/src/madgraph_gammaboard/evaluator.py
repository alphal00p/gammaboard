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
        subprocess_indices: list[int] | str | None = None,
        phase_space: str = "flat",
        channel_index: int = 0,
        channel_indices: list[int] | str | None = None,
        device: str = "cppnone",
        thread_count: int = -1,
        output: list[str] | None = None,
    ) -> None:
        if len(discrete_cardinalities) > 1:
            raise ValueError(
                "MadGraph evaluator supports at most one discrete integrand-index axis"
            )
        self.flattened_integrands = len(discrete_cardinalities) == 1
        self.state = MadSpaceState.load(
            state_path=state_path,
            madgraph_root=madgraph_root,
            subprocess_index=subprocess_index,
            subprocess_indices=subprocess_indices,
            phase_space=phase_space,
            channel_index=channel_index,
            channel_indices=channel_indices,
            device=device,
            thread_count=thread_count,
            flattened_integrands=self.flattened_integrands,
        )
        if int(continuous_dims) != self.state.random_dim:
            raise ValueError(
                f"continuous_dims={continuous_dims} does not match "
                f"{self.state.random_dim} random dimensions required by the MadSpace integrand"
            )
        if (
            self.flattened_integrands
            and int(discrete_cardinalities[0]) != self.state.nr_integrands
        ):
            raise ValueError(
                f"discrete_cardinalities[0]={discrete_cardinalities[0]} does not match "
                f"{self.state.nr_integrands} flattened MadSpace integrands"
            )
        if not self.flattened_integrands and self.state.nr_integrands != 1:
            raise ValueError("continuous-only MadGraph evaluator requires one integrand")
        self.output = output or ["weight"]
        allowed = {"weight"}
        unknown = [name for name in self.output if name not in allowed]
        if unknown:
            raise ValueError(f"unsupported MadGraph output component(s): {unknown!r}")

    @property
    def metadata(self) -> dict:
        return self.state.metadata

    def eval(self, xs_discrete: np.ndarray, xs_continuous: np.ndarray) -> dict[str, np.ndarray]:
        if not self.flattened_integrands:
            if xs_discrete.shape[1] != 0:
                raise ValueError(
                    "single-integrand MadGraph evaluator expects no discrete coordinates"
                )
            weight = self.state.evaluate(xs_continuous)
        else:
            if xs_discrete.shape[1] != 1:
                raise ValueError(
                    "flattened MadGraph evaluator expects one discrete integrand-index coordinate"
                )
            weight = self.state.evaluate(xs_continuous, xs_discrete[:, 0])
        return {name: weight for name in self.output}


def main() -> None:
    run_evaluator(MadGraphEvaluator)


if __name__ == "__main__":
    main()
