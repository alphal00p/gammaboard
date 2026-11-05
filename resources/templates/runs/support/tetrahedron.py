"""The tetrahedron surface evaluated through GammaBoard's per-sample process API."""
import numpy as np
from gammaboard_process import Evaluator, run_evaluator


class Tetrahedron(Evaluator):
    def __init__(self, *, domain, sharpness=6.0, skew=0.0):
        if domain.require_rectangular_shape() != ([], 2):
            raise ValueError("tetrahedron expects two integrand coordinates")
        self.sharpness = sharpness
        self.skew = skew

    def eval(self, xs_discrete, xs_continuous):
        u, v = xs_continuous.T
        a = np.tanh(self.sharpness * np.cos(2 * np.pi * u))
        b = np.tanh(self.sharpness * np.cos(2 * np.pi * (v + self.skew * u)))
        return np.column_stack((a, b, a * b))


if __name__ == "__main__":
    run_evaluator(Tetrahedron)
