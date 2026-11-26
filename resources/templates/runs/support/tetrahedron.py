"""The one-dimensional tetrahedron curve evaluated through GammaBoard's per-sample process API."""

import numpy as np
from gammaboard_process import (
    EvaluationBatch,
    Evaluator,
    EvaluatorDescriptor,
    RectangularDomain,
    run_evaluator,
)


class Tetrahedron(Evaluator):
    def __init__(self, *, domain=None, sharpness=8.0):
        self.domain = domain = domain or RectangularDomain((), 1)
        if domain.require_rectangular_shape() != ([], 1):
            raise ValueError("tetrahedron expects one integrand coordinate")
        self.sharpness = sharpness

    def descriptor(self):
        return EvaluatorDescriptor(self.domain, ["x", "y", "z"])

    def evaluate(self, xs_discrete, xs_continuous, request):
        t = xs_continuous[:, 0]
        a = np.tanh(self.sharpness * np.cos(2 * np.pi * t))
        b = np.tanh(self.sharpness * np.sin(2 * np.pi * t))
        return EvaluationBatch({"x": a, "y": b, "z": a * b})


if __name__ == "__main__":
    run_evaluator(Tetrahedron)
