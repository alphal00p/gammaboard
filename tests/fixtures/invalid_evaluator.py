"""Finite values whose second moment overflows; exercises Rust-side validation."""
from pathlib import Path
import sys

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "process_api/python/src"))
from gammaboard_process import Evaluator, run_evaluator


class OverflowEvaluator(Evaluator):
    def __init__(self, **kwargs):
        pass

    def eval(self, xs_discrete, xs_continuous):
        return np.full(len(xs_continuous), 1e200)


if __name__ == "__main__":
    run_evaluator(OverflowEvaluator)
