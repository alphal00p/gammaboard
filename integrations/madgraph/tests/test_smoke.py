from __future__ import annotations

import json
from unittest.mock import MagicMock, patch

import numpy as np
import pytest

from madgraph_gammaboard.evaluator import MadGraphEvaluator
from madgraph_gammaboard.event_evaluator import (
    MadGraphEventEvaluator,
    gammaloop_state_from_info,
)
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
    state.metadata = {
        "random_dim": random_dim,
        "mapping_random_dim": random_dim - 1,
        "subprocess_index": 0,
        "flavor_index": 0,
        "phase_space": "flat",
        "integrand": "phase_space_mapping+differential_cross_section",
    }
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
            raise AssertionError("expected ValueError for non-empty discrete dimensions")


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


def test_evaluator_exposes_state_metadata() -> None:
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, np.array([1.0]))
        evaluator = MadGraphEvaluator(
            discrete_cardinalities=[],
            continuous_dims=2,
            state_path="/fake",
        )

    assert evaluator.metadata["integrand"] == "phase_space_mapping+differential_cross_section"


def _native_info() -> dict:
    return {
        "status": "done",
        "process": {
            "mean": 3.5,
            "error": 0.25,
            "count": 120,
            "count_opt": 100,
            "count_after_cuts": 90,
            "count_after_cuts_opt": 80,
            "count_unweighted": 42.4,
        },
        "run_times": {
            "survey": {"wall_time_sec": 1.5, "cpu_time_sec": 2.0},
            "generate": {"wall_time_sec": 2.5, "cpu_time_sec": 3.0},
        },
        "histograms": [
            {
                "name": "jet-pt",
                "min": 0.0,
                "max": 20.0,
                "bin_values": [0.1, 1.0, 2.0, 0.2],
                "bin_errors": [0.01, 0.1, 0.2, 0.02],
            }
        ],
    }


def _snapshot_mean_error(state: dict) -> tuple[float, float]:
    count = state["count"]
    total = state["sum_weighted_value"]
    mean = total / count
    variance_numerator = state["sum_sq"] - total * total / count
    error = (variance_numerator / (count * (count - 1))) ** 0.5
    return mean, error


def test_native_info_maps_to_gammaloop_state() -> None:
    state = gammaloop_state_from_info(_native_info())

    real = state["estimate"]["components"][0]["state"]
    mean, error = _snapshot_mean_error(real)
    assert mean == 3.5
    assert abs(error - 0.25) < 1.0e-12
    assert state["diagnostics"]["total_generated_events"] == 80
    assert state["diagnostics"]["total_accepted_events"] == 42
    assert state["diagnostics"]["total_eval_time_ms"] == 4000.0

    histogram = state["bundle"]["histograms"]["jet-pt"]
    assert histogram["sample_count"] == 100
    assert len(histogram["bins"]) == 2
    assert histogram["bins"][0]["x_min"] == 0.0
    assert histogram["bins"][0]["x_max"] == 10.0
    bin_mean, bin_error = _snapshot_mean_error(
        {
            "count": histogram["sample_count"],
            "sum_weighted_value": histogram["bins"][0]["sum_weights"],
            "sum_sq": histogram["bins"][0]["sum_weights_squared"],
        }
    )
    assert bin_mean == 1.0
    assert abs(bin_error - 0.1) < 1.0e-12


def test_native_event_evaluator_runs_generated_state(tmp_path) -> None:
    state_path = tmp_path / "state"
    (state_path / "bin").mkdir(parents=True)
    (state_path / "Events").mkdir()
    (state_path / "bin" / "generate_events").write_text("#!/bin/sh\n")

    def fake_run(*args, **kwargs):
        assert args[0] == ["/usr/bin/python3", str(state_path / "bin/generate_events"), "-f"]
        run_path = state_path / "Events" / "run_01"
        run_path.mkdir()
        (run_path / "info.json").write_text(json.dumps(_native_info()))
        result = MagicMock()
        result.returncode = 0
        result.stdout = ""
        return result

    evaluator = MadGraphEventEvaluator(
        discrete_cardinalities=[],
        continuous_dims=0,
        state_path=str(state_path),
        python="/usr/bin/python3",
    )
    with patch(
        "madgraph_gammaboard.event_evaluator.subprocess.run",
        side_effect=fake_run,
    ):
        result = evaluator.eval_gammaloop(
            np.zeros((1, 0), dtype=np.int64),
            np.zeros((1, 0), dtype=np.float64),
        )

    assert result.state["estimate"]["components"][0]["state"]["count"] == 100
    assert result.training_values is None


def test_native_event_evaluator_explains_stale_madspace(tmp_path) -> None:
    state_path = tmp_path / "state"
    (state_path / "bin").mkdir(parents=True)
    (state_path / "bin" / "generate_events").write_text("#!/bin/sh\n")

    result = MagicMock()
    result.returncode = 1
    result.stdout = "ValueError: Key already present in NamedVector"
    evaluator = MadGraphEventEvaluator(
        discrete_cardinalities=[],
        continuous_dims=0,
        state_path=str(state_path),
        python="/usr/bin/python3",
    )

    with (
        patch(
            "madgraph_gammaboard.event_evaluator.subprocess.run",
            return_value=result,
        ),
        pytest.raises(RuntimeError, match="MadSpace extension is stale"),
    ):
        evaluator.eval_gammaloop(
            np.zeros((1, 0), dtype=np.int64),
            np.zeros((1, 0), dtype=np.float64),
        )
