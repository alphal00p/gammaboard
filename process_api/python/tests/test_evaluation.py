import struct

import numpy as np
import pytest
from gammaboard_process import (
    EvaluationBatch,
    Evaluator,
    EvaluatorDescriptor,
    RectangularDomain,
    runners,
)
from gammaboard_process.evaluation import (
    BINARY_ENCODING,
    DIAGNOSTIC_COUNTS,
    DIAGNOSTIC_TIMES,
)
from gammaboard_process.protocol import PROTOCOL


class Selected(Evaluator):
    def __init__(self, *, domain):
        self.domain = domain or RectangularDomain((), 1)

    def descriptor(self):
        return EvaluatorDescriptor(
            self.domain,
            ["real", "imag"],
            details=True,
            diagnostics=True,
            scalar_projection="abs",
        )

    def evaluate(self, discrete, continuous, request):
        n = len(continuous)
        return EvaluationBatch(
            {"real": continuous[:, 0], "imag": -continuous[:, 0]},
            details=[
                {
                    "integrand_value_re": 2.0,
                    "integrand_value_im": -2.0,
                    "parameterization_jacobian": 3.0,
                }
            ]
            * n
            if request.details
            else None,
            diagnostics=[
                dict.fromkeys(DIAGNOSTIC_COUNTS, 0)
                | dict.fromkeys(DIAGNOSTIC_TIMES, 0.0)
                | {"count_total": 1}
            ]
            * n
            if request.diagnostics
            else None,
        )


def test_discovery_selection_and_optional_channels(monkeypatch):
    monkeypatch.setattr(runners, "ProtocolIo", lambda: None)
    worker = runners._EvaluatorWorker(Selected)
    reply = worker.handle(
        "initialize", {"protocol": PROTOCOL, "role": "evaluator", "args": {}}
    )
    assert reply["descriptor"]["domain"] == RectangularDomain((), 1).to_dict()
    assert reply["evaluation_protocol"] == "gammaboard-evaluation-v1"
    payload = np.asarray([6.0], dtype="<f8").tobytes()
    request = {
        "nr_samples": 1,
        "outputs": {
            "components": ["imag", "real"],
            "details": True,
            "diagnostics": True,
        },
    }
    rows = worker.handle("evaluate", request, payload)["samples"]
    assert rows[0]["values"] == [-6.0, 6.0]
    assert rows[0]["details"]["parameterization_jacobian"] == 3.0
    assert rows[0]["diagnostics"]["count_total"] == 1
    assert "total_canonical_sampling_preparation_time_ms" not in rows[0]["diagnostics"]
    binary = worker.handle(
        "evaluate", request | {"encoding": BINARY_ENCODING}, payload
    )["__binary__"]
    assert struct.unpack_from("<4dBd", binary) == (-6.0, 6.0, 2.0, -2.0, 1, 3.0)
    assert struct.unpack_from("<q", binary, 41) == (1,)
    assert binary[-2:] == b"\0\0"  # unavailable optional timings, not known zero
    values = worker.handle(
        "evaluate", {"nr_samples": 1, "outputs": {"components": ["real"]}}, payload
    )
    assert values == {"samples": [{"values": [6.0]}]}
    with pytest.raises(ValueError, match="unsupported"):
        worker.handle(
            "evaluate",
            {"nr_samples": 1, "outputs": {"components": ["real"], "observables": True}},
            payload,
        )
