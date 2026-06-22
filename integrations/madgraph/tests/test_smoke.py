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


def test_madgraph_python_matrix_backend(tmp_path: Path) -> None:
    (tmp_path / "matrix_demo.py").write_text(
        """
class Matrix_demo:
    def smatrix(self, p, model, flavor=None):
        assert p.shape == (4, 4)
        assert model.normalization == 3.0
        assert flavor == [11, -11, 13, -13]
        return model.normalization
""".lstrip()
    )
    (tmp_path / "model_demo.py").write_text(
        """
normalization = 3.0
""".lstrip()
    )
    evaluator = MadGraphEvaluator(
        discrete_cardinalities=[],
        continuous_dims=2,
        ecm=10.0,
        output=["matrix_element"],
        phase_space={"kind": "two_body", "final_state_masses": [0.0, 0.0], "include_flux": True},
        matrix_element={
            "kind": "madgraph_python_matrix",
            "module": "matrix_demo",
            "model_module": "model_demo",
            "pdgs": [11, -11, 13, -13],
            "search_path": str(tmp_path),
        },
    )

    values = evaluator.eval(np.zeros((2, 0), dtype=np.int64), np.full((2, 2), 0.5))

    np.testing.assert_allclose(values["matrix_element"], [3.0, 3.0])


def test_madgraph_f2py_subprocess_backend(tmp_path: Path) -> None:
    (tmp_path / "matrix2py.py").write_text(
        """
initialized = None

def py_initialisemodel(path):
    global initialized
    initialized = path

def py_smatrix(p):
    assert initialized == "param_card.dat"
    assert p.shape == (4, 4)
    return 7.0

def py_get_value(p, alphas, nhel):
    assert initialized == "param_card.dat"
    assert p.shape == (4, 4)
    assert alphas == 0.12
    assert nhel == -1
    return 8.0

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
        output=["matrix_element"],
        phase_space={"kind": "two_body", "final_state_masses": [0.0, 0.0], "include_flux": True},
        matrix_element={
            "kind": "madgraph_f2py_subprocess",
            "module": "matrix2py",
            "search_path": str(tmp_path),
            "initialize_path": "param_card.dat",
            "function": "py_matrix_sum",
            "flavor": [1, 1, 2, 2],
            "helicities": [[1, 1, 1, 1], [1, -1, 1, -1]],
            "normalization": 2.0,
        },
    )

    values = evaluator.eval(np.zeros((2, 0), dtype=np.int64), np.full((2, 2), 0.5))

    np.testing.assert_allclose(values["matrix_element"], [2.0, 2.0])


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
            phase_space={"kind": "two_body", "final_state_masses": [0.0, 0.0], "include_flux": True},
            matrix_element={
                "kind": "madgraph_f2py_subprocess",
                "module": "matrix2py",
                "search_path": str(tmp_path),
                "function": "py_smatrix",
            },
        )
    except ValueError as error:
        assert "py_smatrix" in str(error)
    else:
        raise AssertionError("expected py_smatrix to be rejected")
