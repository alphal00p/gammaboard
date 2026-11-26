from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

import numpy as np
import pytest
import tomllib
from gammaboard_process import EvaluationRequest, RectangularDomain

ROOT = Path(__file__).resolve().parents[3]
CARDS = ROOT / "resources/templates/runs"


def evaluator(**kwargs):
    spec = spec_from_file_location("tetrahedron_showcase", CARDS / "support/tetrahedron.py")
    module = module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.Tetrahedron(**kwargs)


def evaluate(provider, points):
    request = EvaluationRequest(["x", "y", "z"])
    result = provider.evaluate(np.empty((len(points), 0), dtype=np.int64), points, request)
    return np.column_stack([result.values[c] for c in request.components])


def test_curve_visits_the_four_specified_vertices_and_preserves_component_order():
    points = np.array([[1 / 8], [7 / 8], [3 / 8], [5 / 8]])
    values = evaluate(evaluator(), points)
    assert values.shape == (4, 3)
    centers = np.array([[1, 1, 1], [1, -1, -1], [-1, 1, -1], [-1, -1, 1]])
    np.testing.assert_allclose(values, centers, atol=5e-5)
    np.testing.assert_allclose(values[:, 0], np.tanh(8 * np.cos(2 * np.pi * points[:, 0])))
    np.testing.assert_allclose(values[:, 1], np.tanh(8 * np.sin(2 * np.pi * points[:, 0])))
    np.testing.assert_allclose(values[:, 2], values[:, 0] * values[:, 1])


def test_curve_is_closed_has_zero_integrals_and_requires_one_input():
    provider = evaluator(domain=RectangularDomain((), 1))
    ends = evaluate(provider, np.array([[0.0], [1.0]]))
    np.testing.assert_allclose(ends[0], ends[1], atol=2e-15)
    grid = (np.arange(16384) + 0.5)[:, None] / 16384
    np.testing.assert_allclose(evaluate(provider, grid).mean(axis=0), 0.0, atol=1e-14)
    with pytest.raises(ValueError, match="one integrand coordinate"):
        evaluator(domain=RectangularDomain((), 2))


def test_native_comparison_changes_only_partition_and_estimator():
    def card(name):
        return tomllib.loads((CARDS / f"{name}.toml").read_text())

    stratified = card("tetrahedron-native")
    ordinary = card("tetrahedron-unstratified")
    assert stratified["evaluator"]["args"] == ["t"]
    assert stratified["replacements"]["sharpness"] == 8
    assert stratified["replacements"]["train_samples"] == 1048576
    assert stratified["replacements"]["sample_samples"] == 131072
    partition = stratified["task_queue"][0]["sampler"]["config"].pop("stratification")
    assert list(partition["centers"].values()) == [[1, 1, 1], [1, -1, -1], [-1, 1, -1], [-1, -1, 1]]
    assert stratified["task_queue"][1]["estimator"] == "stratified"
    assert ordinary["task_queue"][1]["estimator"] == "pooled"
    stratified["task_queue"][1]["estimator"] = "pooled"
    stratified.pop("name")
    ordinary.pop("name")
    assert stratified == ordinary
    for task in stratified["task_queue"][2:]:
        assert task["geometry"]["offset"] == [0.0]
        assert task["geometry"]["direction"] == [1.0]


def test_campaign_budget_covers_both_replicas_training_and_inference():
    child = tomllib.loads((CARDS / "support/tetrahedron-trial.toml").read_text())
    campaign = tomllib.loads((CARDS / "tetrahedron-campaign.toml").read_text())
    budget = child["replacements"]["train_samples"] + child["replacements"]["sample_samples"]
    assert campaign["stop_condition"]["max_total_samples"] >= len(campaign["children"]) * budget
