from __future__ import annotations

import json
import math
import subprocess
from pathlib import Path
from typing import Any

import numpy as np
from gammaboard_process import Evaluator, GammaLoopBatchResult, log, run_evaluator


class MadGraphEventEvaluator(Evaluator):
    """Run native MG7 event generation and expose its statistics as observables."""

    def __init__(
        self,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        state_path: str,
        python: str | None = None,
    ) -> None:
        if discrete_cardinalities:
            raise ValueError("MadGraph event generation does not use discrete dimensions")
        if int(continuous_dims) != 0:
            raise ValueError(
                "MadGraph event generation expects a zero-dimensional trigger domain"
            )
        self.state_path = Path(state_path).expanduser().resolve()
        if not self.state_path.is_dir():
            raise ValueError(f"MadGraph state_path does not exist: {self.state_path}")
        self.command = self.state_path / "bin" / "generate_events"
        if not self.command.is_file():
            raise ValueError(
                f"MadGraph state does not contain bin/generate_events: {self.command}"
            )
        self.python = python
        self._completed = False

    @property
    def metadata(self) -> dict[str, Any]:
        return {
            "integrand": "native_madgraph_event_generation",
            "state_path": str(self.state_path),
        }

    def eval(
        self, xs_discrete: np.ndarray, xs_continuous: np.ndarray
    ) -> np.ndarray:
        raise RuntimeError(
            "MadGraphEventEvaluator requires evaluator accumulator = 'gammaloop'"
        )

    def eval_gammaloop(
        self, xs_discrete: np.ndarray, xs_continuous: np.ndarray
    ) -> GammaLoopBatchResult:
        if len(xs_continuous) != 1:
            raise ValueError(
                "MadGraph event generation must be triggered by a one-sample task"
            )
        if self._completed:
            raise RuntimeError("MadGraph event generation was already executed")

        info_path = self._run_generation()
        with info_path.open(encoding="utf-8") as handle:
            info = json.load(handle)
        if info.get("status") != "done":
            raise RuntimeError(
                f"MadGraph generation did not finish successfully: {info.get('status')!r}"
            )
        self._completed = True
        return GammaLoopBatchResult(state=gammaloop_state_from_info(info))

    def _run_generation(self) -> Path:
        events_path = self.state_path / "Events"
        events_path.mkdir(exist_ok=True)
        existing = {path.resolve() for path in events_path.iterdir() if path.is_dir()}
        command = [str(self.command), "-f"]
        if self.python:
            command.insert(0, self.python)
        result = subprocess.run(
            command,
            cwd=self.state_path,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            check=False,
        )
        output_tail = "\n".join(result.stdout.splitlines()[-40:])
        if result.returncode != 0:
            hint = ""
            if "Key already present in NamedVector" in result.stdout:
                hint = (
                    "\nThe bundled MadSpace extension is stale. Rebuild it from the "
                    "current MadGraph source with: "
                    "/usr/bin/python3 madspace/install.py --source"
                )
            raise RuntimeError(
                f"native MadGraph event generation exited with code {result.returncode}"
                + (f"\n{output_tail}" if output_tail else "")
                + hint
            )
        if output_tail:
            log(output_tail)
        created = sorted(
            (
                path.resolve()
                for path in events_path.iterdir()
                if path.is_dir() and path.resolve() not in existing
            ),
            key=lambda path: path.stat().st_mtime_ns,
        )
        if len(created) != 1:
            raise RuntimeError(
                "native MadGraph event generation must create exactly one run directory; "
                f"found {len(created)}"
            )
        info_path = created[0] / "info.json"
        if not info_path.is_file():
            raise RuntimeError(f"MadGraph generation did not produce {info_path}")
        return info_path


def gammaloop_state_from_info(info: dict[str, Any]) -> dict[str, Any]:
    process = info.get("process")
    if not isinstance(process, dict):
        raise ValueError("MadGraph info.json is missing process statistics")

    sample_count = max(2, int(process.get("count_opt", process.get("count", 0))))
    mean = _finite_float(process.get("mean", 0.0), "process.mean")
    error = max(0.0, _finite_float(process.get("error", 0.0), "process.error"))
    real_state = _scalar_state_from_mean_error(sample_count, mean, error)
    zero_state = _scalar_state_from_mean_error(sample_count, 0.0, 0.0)

    histograms: dict[str, Any] = {}
    for raw in info.get("histograms") or []:
        histogram = _histogram_snapshot(raw, sample_count, process)
        name = str(raw["name"])
        if name in histograms:
            raise ValueError(f"duplicate MadGraph histogram name: {name!r}")
        histograms[name] = histogram

    total_time_ms = (
        sum(
            max(0.0, float(value.get("wall_time_sec", 0.0)))
            for value in (info.get("run_times") or {}).values()
            if isinstance(value, dict)
        )
        * 1000.0
    )
    count_total = int(process.get("count", 0))
    generated_events = int(
        process.get("count_after_cuts_opt", process.get("count_after_cuts", 0))
    )
    accepted_events = int(round(float(process.get("count_unweighted", 0.0))))

    return {
        "bundle": {"histograms": histograms},
        "estimate": {
            "components": [
                {"name": "real", "state": real_state},
                {"name": "imag", "state": zero_state},
            ],
            "projection": {
                "name": "training_projection",
                "state": real_state,
            },
            "projection_spec": {"kind": "norm"},
        },
        "diagnostics": {
            "count_total": count_total,
            "count_double_precision": count_total,
            "count_quad_precision": 0,
            "count_arb_precision": 0,
            "count_nan": 0,
            "count_nan_or_unstable": 0,
            "count_loop_momenta_escalated": 0,
            "total_eval_time_ms": total_time_ms,
            "total_integrand_eval_time_ms": total_time_ms,
            "total_evaluator_eval_time_ms": 0.0,
            "total_parameterization_time_ms": 0.0,
            "total_event_processing_time_ms": total_time_ms,
            "total_generated_events": generated_events,
            "total_accepted_events": accepted_events,
        },
    }


def _histogram_snapshot(
    raw: dict[str, Any], sample_count: int, process: dict[str, Any]
) -> dict[str, Any]:
    name = str(raw["name"])
    x_min = _finite_float(raw["min"], f"histogram {name}.min")
    x_max = _finite_float(raw["max"], f"histogram {name}.max")
    values = [_finite_float(value, f"histogram {name}.bin_values") for value in raw["bin_values"]]
    errors = [
        max(0.0, _finite_float(value, f"histogram {name}.bin_errors"))
        for value in raw["bin_errors"]
    ]
    if x_max <= x_min:
        raise ValueError(f"histogram {name!r} max must be greater than min")
    if len(values) != len(errors) or len(values) < 3:
        raise ValueError(
            f"histogram {name!r} must have matching bin arrays including under/overflow"
        )
    bin_count = len(values) - 2
    width = (x_max - x_min) / bin_count

    def bin_snapshot(index: int, lower: float | None, upper: float | None) -> dict[str, Any]:
        state = _scalar_state_from_mean_error(
            sample_count, values[index], errors[index]
        )
        return {
            "x_min": lower,
            "x_max": upper,
            "bin_id": None,
            "label": None,
            "entry_count": 0,
            "sum_weights": state["sum_weighted_value"],
            "sum_weights_squared": state["sum_sq"],
            "mitigated_fill_count": 0,
        }

    bins = [
        bin_snapshot(
            index + 1,
            x_min + index * width,
            x_min + (index + 1) * width,
        )
        for index in range(bin_count)
    ]
    return {
        "kind": "continuous",
        "title": name,
        "type_description": "HwU",
        "phase": "real",
        "value_transform": "identity",
        "supports_misbinning_mitigation": False,
        "x_min": x_min,
        "x_max": x_max,
        "sample_count": sample_count,
        "log_x_axis": False,
        "log_y_axis": False,
        "discrete_min_bin_id": None,
        "discrete_ordering": None,
        "bins": bins,
        "underflow_bin": bin_snapshot(0, None, x_min),
        "overflow_bin": bin_snapshot(len(values) - 1, x_max, None),
        "statistics": {
            "in_range_entry_count": int(
                process.get("count_after_cuts_opt", process.get("count_after_cuts", 0))
            ),
            "nan_value_count": 0,
            "mitigated_pair_count": 0,
        },
    }


def _scalar_state_from_mean_error(
    count: int, mean: float, error: float
) -> dict[str, Any]:
    count = max(2, int(count))
    total = mean * count
    sum_sq = error * error * count * (count - 1) + total * total / count
    return {
        "count": count,
        "sum_weighted_value": total,
        "sum_abs": abs(total),
        "sum_sq": sum_sq,
    }


def _finite_float(value: Any, label: str) -> float:
    result = float(value)
    if not math.isfinite(result):
        raise ValueError(f"{label} must be finite")
    return result


def main() -> None:
    run_evaluator(MadGraphEventEvaluator)


if __name__ == "__main__":
    main()
