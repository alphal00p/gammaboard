from __future__ import annotations

from unittest.mock import MagicMock, patch

import numpy as np

from madgraph_gammaboard.evaluator import MadGraphEvaluator
from madgraph_gammaboard.state import MadSpaceIntegrand, MadSpaceState


class _Runtime:
    def __init__(self, offset: float) -> None:
        self.offset = offset

    def call(self, inputs: list[np.ndarray]) -> list[np.ndarray]:
        xs = inputs[0]
        return [xs[:, 0] + self.offset]


def test_state_rejects_missing_dir() -> None:
    try:
        MadSpaceState.load(state_path="/nonexistent/madspace_state")
    except ValueError as error:
        assert "does not exist" in str(error)
    else:
        raise AssertionError("expected ValueError for missing state_path")


def _fake_state(
    random_dim: int, weights: np.ndarray, nr_integrands: int = 1
) -> MagicMock:
    state = MagicMock(spec=MadSpaceState)
    state.random_dim = random_dim
    state.nr_integrands = nr_integrands
    state.metadata = {"random_dim": random_dim, "nr_integrands": nr_integrands}
    state.evaluate.return_value = weights
    return state


def test_evaluator_rejects_too_many_discrete_dims() -> None:
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, np.array([]))
        try:
            MadGraphEvaluator(
                discrete_cardinalities=[2, 3],
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


def test_evaluator_flattened_integrands_use_discrete_index() -> None:
    weights = np.array([2.0, 3.0])
    state = _fake_state(2, weights, nr_integrands=3)
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = state
        evaluator = MadGraphEvaluator(
            discrete_cardinalities=[3],
            continuous_dims=2,
            state_path="/fake",
            subprocess_indices="all",
            channel_indices="all",
        )
        result = evaluator.eval(
            np.array([[0], [2]], dtype=np.int64),
            np.full((2, 2), 0.5),
        )

    mock_cls.load.assert_called_once()
    assert mock_cls.load.call_args.kwargs["flattened_integrands"] is True
    state.evaluate.assert_called_once()
    np.testing.assert_array_equal(state.evaluate.call_args.args[1], np.array([0, 2]))
    np.testing.assert_allclose(result["weight"], weights)


def test_evaluator_flattened_single_integrand_still_uses_discrete_index() -> None:
    weights = np.array([2.0])
    state = _fake_state(2, weights, nr_integrands=1)
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = state
        evaluator = MadGraphEvaluator(
            discrete_cardinalities=[1],
            continuous_dims=2,
            state_path="/fake",
        )
        result = evaluator.eval(
            np.array([[0]], dtype=np.int64),
            np.full((1, 2), 0.5),
        )

    state.evaluate.assert_called_once()
    np.testing.assert_array_equal(state.evaluate.call_args.args[1], np.array([0]))
    np.testing.assert_allclose(result["weight"], weights)


def test_evaluator_rejects_flattened_integrand_cardinality_mismatch() -> None:
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, np.array([]), nr_integrands=3)
        try:
            MadGraphEvaluator(
                discrete_cardinalities=[2],
                continuous_dims=2,
                state_path="/fake",
            )
        except ValueError as error:
            assert "discrete_cardinalities" in str(error)
        else:
            raise AssertionError("expected ValueError for cardinality mismatch")


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


def test_evaluator_exposes_state_metadata() -> None:
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, np.array([1.0]), nr_integrands=1)
        evaluator = MadGraphEvaluator(
            discrete_cardinalities=[],
            continuous_dims=2,
            state_path="/fake",
        )

    assert evaluator.metadata == {"random_dim": 2, "nr_integrands": 1}


def test_state_evaluate_flattened_integrands_scales_uniform_channel_choice() -> None:
    state = MadSpaceState(
        random_dim=2,
        integrands=[
            MadSpaceIntegrand(0, 0, _Runtime(10.0)),
            MadSpaceIntegrand(0, 1, _Runtime(20.0)),
        ],
    )

    values = state.evaluate(
        np.array([[1.0, 0.0], [2.0, 0.0], [3.0, 0.0]]),
        np.array([0, 1, 0]),
    )

    np.testing.assert_allclose(values, np.array([22.0, 44.0, 26.0]))
