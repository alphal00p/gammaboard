from __future__ import annotations

import numpy as np

from gammaboard_process import Evaluator, ObservableBatch

N_BINS = 20
X_MIN, X_MAX = 0.0, 1.0


def _f(x: np.ndarray) -> np.ndarray:
    """Toy integrand f(x) = 6x(1-x), integral = 1."""
    return 6.0 * x * (1.0 - x)


def _histogram_snapshot(xs: np.ndarray, weights: np.ndarray, mask: np.ndarray) -> dict:
    bin_width = (X_MAX - X_MIN) / N_BINS
    bins = [
        {
            "x_min": X_MIN + i * bin_width,
            "x_max": X_MIN + (i + 1) * bin_width,
            "bin_id": None,
            "label": None,
            "entry_count": 0,
            "sum_weights": 0.0,
            "sum_weights_squared": 0.0,
            "mitigated_fill_count": 0,
        }
        for i in range(N_BINS)
    ]
    underflow = {
        "x_min": None,
        "x_max": None,
        "bin_id": None,
        "label": None,
        "entry_count": 0,
        "sum_weights": 0.0,
        "sum_weights_squared": 0.0,
        "mitigated_fill_count": 0,
    }
    overflow = {**underflow}
    in_range = 0
    for x, w, matched in zip(xs.tolist(), weights.tolist(), mask.tolist()):
        if not matched:
            continue
        idx = int((x - X_MIN) / bin_width)
        if idx < 0:
            underflow["entry_count"] += 1
            underflow["sum_weights"] += w
            underflow["sum_weights_squared"] += w * w
        elif idx >= N_BINS:
            overflow["entry_count"] += 1
            overflow["sum_weights"] += w
            overflow["sum_weights_squared"] += w * w
        else:
            bins[idx]["entry_count"] += 1
            bins[idx]["sum_weights"] += w
            bins[idx]["sum_weights_squared"] += w * w
            in_range += 1
    return {
        "kind": "continuous",
        "title": "x",
        "type_description": "HwU",
        "phase": "real",
        "value_transform": "identity",
        "supports_misbinning_mitigation": False,
        "x_min": X_MIN,
        "x_max": X_MAX,
        "sample_count": len(xs),
        "log_x_axis": False,
        "log_y_axis": False,
        "discrete_min_bin_id": None,
        "discrete_ordering": None,
        "bins": bins,
        "underflow_bin": underflow,
        "overflow_bin": overflow,
        "statistics": {
            "in_range_entry_count": in_range,
            "nan_value_count": 0,
            "mitigated_pair_count": 0,
        },
    }


class PolynomialEvaluator(Evaluator):
    """f(x)=6x(1-x) with one unweighted histogram contribution per draw."""

    def __init__(self, *, domain):
        domain.require_rectangular_shape()

    def eval(self, xs_discrete, xs_continuous):
        x = xs_continuous[:, 0]
        values = _f(x)
        return ObservableBatch(
            values=np.column_stack((values, np.zeros(len(x)))),
            observables=[
                {
                    "histograms": {
                        "x": _histogram_snapshot(
                            np.asarray([position]),
                            np.asarray([value]),
                            np.asarray([True]),
                        )
                    }
                }
                for position, value in zip(x, values)
            ],
        )
