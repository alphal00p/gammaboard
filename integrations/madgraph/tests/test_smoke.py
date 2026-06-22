from __future__ import annotations

from pathlib import Path

import numpy as np

from madgraph_gammaboard.evaluator import MadGraphEvaluator


def test_madgraph_f2py_subprocess_backend(tmp_path: Path) -> None:
    (tmp_path / "matrix2py.py").write_text(
        """
initialized = None

def py_initialisemodel(path):
    global initialized
    initialized = path

def py_matrix(p, nhel, flavor):
    assert initialized == "param_card.dat"
    assert p.shape == (4, 4)
    assert list(flavor) == [1, 1, 2, 2]
    return float(sum(nhel))
""".lstrip()
    )
    evaluator = MadGraphEvaluator(
        discrete_cardinalities=[],
        continuous_dims=2,
        ecm=10.0,
        output=["value", "matrix_element", "phase_space_weight"],
        phase_space={
            "kind": "two_body",
            "final_state_masses": [0.0, 0.0],
            "include_flux": True,
        },
        matrix_element={
            "module": "matrix2py",
            "search_path": str(tmp_path),
            "initialize_path": "param_card.dat",
            "function": "py_matrix_sum",
            "flavor": [1, 1, 2, 2],
            "helicities": [[1, 1, 1, 1], [1, -1, 1, -1]],
            "normalization": 2.0,
        },
    )

    values = evaluator.eval(
        np.zeros((2, 0), dtype=np.int64),
        np.full((2, 2), 0.5),
    )

    assert set(values) == {"value", "matrix_element", "phase_space_weight"}
    np.testing.assert_allclose(values["matrix_element"], [2.0, 2.0])
    np.testing.assert_allclose(values["value"], 2.0 * values["phase_space_weight"])


def test_madgraph_f2py_subprocess_rejects_unsafe_smatrix(tmp_path: Path) -> None:
    (tmp_path / "matrix2py.py").write_text(
        """
def py_smatrix(p):
    return 7.0
""".lstrip()
    )

    try:
        MadGraphEvaluator(
            discrete_cardinalities=[],
            continuous_dims=2,
            ecm=10.0,
            output=["matrix_element"],
            phase_space={
                "kind": "two_body",
                "final_state_masses": [0.0, 0.0],
                "include_flux": True,
            },
            matrix_element={
                "module": "matrix2py",
                "search_path": str(tmp_path),
                "function": "py_smatrix",
            },
        )
    except ValueError as error:
        assert "py_smatrix" in str(error)
    else:
        raise AssertionError("expected py_smatrix to be rejected")
