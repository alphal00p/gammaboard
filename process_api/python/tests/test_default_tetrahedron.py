from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

import numpy as np
from gammaboard_process import RectangularDomain, EvaluationRequest


def test_default_process_evaluator_returns_three_unweighted_components():
    path = (
        Path(__file__).resolve().parents[3]
        / "resources/templates/runs/support/tetrahedron.py"
    )
    spec = spec_from_file_location("tetrahedron_showcase", path)
    module = module_from_spec(spec)
    spec.loader.exec_module(module)
    points = np.array([[0.0, 0.0], [0.0, 0.5], [0.5, 0.0], [0.5, 0.5]])
    evaluator = module.Tetrahedron(domain=RectangularDomain((), 2))
    request = EvaluationRequest(["x", "y", "z"])
    result = evaluator.evaluate(np.empty((4, 0), dtype=np.int64), points, request)
    values = np.column_stack([result.values[c] for c in request.components])
    assert values.shape == (4, 3)
    np.testing.assert_allclose(values[:, 2], values[:, 0] * values[:, 1])
    np.testing.assert_allclose(values.mean(axis=0), 0.0, atol=1e-14)
    np.testing.assert_allclose(abs(values[0, :2]), np.tanh(6.0))
