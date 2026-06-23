from __future__ import annotations

from unittest.mock import MagicMock, patch

import numpy as np

from madgraph_gammaboard.evaluator import MadGraphEvaluator
from madgraph_gammaboard.state import MadSpaceState


def test_state_rejects_missing_dir() -> None:
    try:
        MadSpaceState.load(state_path="/nonexistent/madspace_state")
    except ValueError as error:
        assert "does not exist" in str(error)
    else:
        raise AssertionError("expected ValueError for missing state_path")


def _fake_state(random_dim: int, weights: np.ndarray) -> MagicMock:
    state = MagicMock(spec=MadSpaceState)
    state.random_dim = random_dim
    state.evaluate.return_value = weights
    return state


def test_evaluator_rejects_discrete_dims() -> None:
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, np.array([]))
        try:
            MadGraphEvaluator(
                discrete_cardinalities=[2],
                continuous_dims=2,
                state_path="/fake",
            )
        except ValueError as error:
            assert "discrete" in str(error)
        else:
            raise AssertionError("expected ValueError for non-empty discrete_cardinalities")


def test_evaluator_rejects_wrong_continuous_dims() -> None:
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(4, np.array([]))
        try:
            MadGraphEvaluator(
                discrete_cardinalities=[],
                continuous_dims=2,
                state_path="/fake",
            )
        except ValueError as error:
            assert "continuous_dims" in str(error)
        else:
            raise AssertionError("expected ValueError for mismatched continuous_dims")


def test_evaluator_rejects_unknown_output() -> None:
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, np.array([]))
        try:
            MadGraphEvaluator(
                discrete_cardinalities=[],
                continuous_dims=2,
                state_path="/fake",
                output=["value"],
            )
        except ValueError as error:
            assert "unsupported" in str(error)
        else:
            raise AssertionError("expected ValueError for unknown output component")


def test_evaluator_eval_returns_weight() -> None:
    weights = np.array([0.5, 0.7])
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, weights)
        evaluator = MadGraphEvaluator(
            discrete_cardinalities=[],
            continuous_dims=2,
            state_path="/fake",
            output=["weight"],
        )
        result = evaluator.eval(
            np.zeros((2, 0), dtype=np.int64),
            np.full((2, 2), 0.5),
        )

    assert set(result) == {"weight"}
    np.testing.assert_allclose(result["weight"], weights)


def test_evaluator_default_output_is_weight() -> None:
    weights = np.array([1.0])
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, weights)
        evaluator = MadGraphEvaluator(
            discrete_cardinalities=[],
            continuous_dims=2,
            state_path="/fake",
        )
        result = evaluator.eval(
            np.zeros((1, 0), dtype=np.int64),
            np.full((1, 2), 0.3),
        )

    assert set(result) == {"weight"}
    np.testing.assert_allclose(result["weight"], weights)
