from __future__ import annotations
from gammaboard_process.domain import RectangularDomain

import os
import subprocess
from unittest.mock import MagicMock, patch

import numpy as np
import pytest

from madgraph_gammaboard.evaluator import MadGraphEvaluator
from madgraph_gammaboard.state import MadSpaceState


PINNED_MADGRAPH_REVISION = "920d224232b24a3a736443986e2040369929298f"


def test_state_rejects_missing_dir() -> None:
    with pytest.raises(ValueError, match="does not exist"):
        MadSpaceState.load(state_path="/nonexistent/madspace_state")


def test_pinned_checkout_with_user_writable_lhapdf_data() -> None:
    """Exercise a real documented MG7 state when CI provides the optional assets."""
    madgraph_root = os.environ.get("GAMMABOARD_MADGRAPH_ROOT")
    state_path = os.environ.get("GAMMABOARD_MADGRAPH_STATE")
    lhapdf_data = os.environ.get("LHAPDF_DATA_PATH")
    if not (madgraph_root and state_path and lhapdf_data):
        pytest.skip("optional pinned MadGraph checkout/state not configured")

    revision = subprocess.check_output(
        ["git", "-C", madgraph_root, "rev-parse", "HEAD"], text=True
    ).strip()
    assert revision == PINNED_MADGRAPH_REVISION
    assert os.access(lhapdf_data, os.W_OK)

    state = MadSpaceState.load(
        state_path=state_path,
        madgraph_root=madgraph_root,
        subprocess_index=0,
        flavor_index=0,
    )
    evaluator = MadGraphEvaluator(
        domain=RectangularDomain(tuple([]), state.random_dim),
        state_path=state_path,
        madgraph_root=madgraph_root,
        output=["weight"],
    )
    result = evaluator.eval(
        np.zeros((1, 0), dtype=np.int64),
        np.full((1, state.random_dim), 0.5),
    )
    assert np.isfinite(result["weight"]).all()


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


@pytest.mark.parametrize(
    ("random_dim", "kwargs", "message"),
    [
        (2, {"discrete_cardinalities": [2], "continuous_dims": 2}, "discrete"),
        (4, {"discrete_cardinalities": [], "continuous_dims": 2}, "continuous_dims"),
        (
            2,
            {"discrete_cardinalities": [], "continuous_dims": 2, "output": ["value"]},
            "unsupported",
        ),
    ],
)
def test_evaluator_rejects_invalid_config(
    random_dim: int, kwargs: dict, message: str
) -> None:
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(random_dim, np.array([]))
        with pytest.raises(ValueError, match=message):
            MadGraphEvaluator(
                state_path="/fake",
                domain=RectangularDomain(
                    tuple(kwargs["discrete_cardinalities"]), kwargs["continuous_dims"]
                ),
                **{
                    k: v
                    for k, v in kwargs.items()
                    if k not in ("discrete_cardinalities", "continuous_dims")
                },
            )


def test_evaluator_eval_returns_weight() -> None:
    weights = np.array([0.5, 0.7])
    with patch("madgraph_gammaboard.evaluator.MadSpaceState") as mock_cls:
        mock_cls.load.return_value = _fake_state(2, weights)
        evaluator = MadGraphEvaluator(
            domain=RectangularDomain(tuple([]), 2),
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
            domain=RectangularDomain(tuple([]), 2),
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
            domain=RectangularDomain(tuple([]), 2),
            state_path="/fake",
        )

    assert (
        evaluator.metadata()["integrand"]
        == "phase_space_mapping+differential_cross_section"
    )
