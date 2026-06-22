from __future__ import annotations

from pathlib import Path

import numpy as np

from madgraph_gammaboard.evaluator import MadGraphEvaluator


def test_python_callable_two_body_evaluator(tmp_path: Path) -> None:
    adapter = tmp_path / "adapter.py"
    adapter.write_text(
        """
import numpy as np

def matrix_element(momenta, parameters=None):
    return np.full((momenta.shape[0],), 2.0)
""".lstrip()
    )
    evaluator = MadGraphEvaluator(
        discrete_cardinalities=[],
        continuous_dims=2,
        ecm=10.0,
        output=["value", "matrix_element", "phase_space_weight"],
        phase_space={"kind": "two_body", "final_state_masses": [0.0, 0.0], "include_flux": True},
        matrix_element={
            "kind": "python_callable",
            "module": "adapter",
            "function": "matrix_element",
            "search_path": str(tmp_path),
        },
    )

    values = evaluator.eval(
        np.zeros((3, 0), dtype=np.int64),
        np.array([[0.0, 0.0], [0.5, 0.5], [1.0, 0.25]], dtype=np.float64),
    )

    assert set(values) == {"value", "matrix_element", "phase_space_weight"}
    np.testing.assert_allclose(values["matrix_element"], [2.0, 2.0, 2.0])
    np.testing.assert_allclose(values["value"], 2.0 * values["phase_space_weight"])
