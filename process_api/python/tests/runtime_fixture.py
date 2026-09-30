"""Shared correctness/overhead fixture, served by the production Python SDK."""
import atexit
from functools import wraps
import json
import os
from pathlib import Path
import sys
import time

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'src'))
from gammaboard_process import GenerationStatus, SampleBatch, run_evaluator, run_sampler


def timed(method):
    @wraps(method)
    def call(self, *args):
        start = time.perf_counter()
        result = method(self, *args)
        elapsed = time.perf_counter() - start
        self.timings.setdefault(method.__name__, []).append(elapsed)
        return result
    return call


class Fixture:
    def __init__(self, *, continuous_dims, discrete_cardinalities, work=0,
                 stats_path=None, evaluator_metadata=None):
        self.dims = continuous_dims
        self.cards = discrete_cardinalities
        self.work = work
        self.timings = {}
        self.count = 0
        self.total = 0.
        if stats_path:
            atexit.register(lambda: Path(stats_path).write_text(json.dumps(dict(
                timings=self.timings, cpus=sorted(os.sched_getaffinity(0)),
                numpy_version=np.__version__))))

    def transform(self, values):
        values = np.array(values, dtype=np.float64, copy=True)
        for _ in range(self.work):
            np.sin(values, out=values)
        return values

    @timed
    def eval(self, xs_discrete, xs_continuous):
        values = self.transform(xs_continuous[:, 0])
        if self.cards:
            values += xs_discrete.sum(axis=1)
        return values

    @timed
    def generate(self, remaining_sample_budget):
        if remaining_sample_budget == 0:
            return GenerationStatus.FINISHED
        nr_samples = min(1_048_576, remaining_sample_budget if remaining_sample_budget is not None else 1_048_576)
        values = self.transform(np.full(nr_samples, .5))
        discrete = np.empty((nr_samples, len(self.cards)), dtype=np.int64)
        for i, cardinality in enumerate(self.cards):
            discrete[:, i] = np.arange(nr_samples) % cardinality
        return SampleBatch(discrete, np.repeat(values[:, None], self.dims, axis=1),
                           np.full(nr_samples, 2.), training_remaining=10**12 - self.count)

    @timed
    def feedback(self, values):
        self.count += len(values)
        self.total += float(self.transform(values).sum())

    def snapshot(self):
        return self.get_diagnostics()

    def get_diagnostics(self):
        return dict(count=self.count, total=self.total)


if __name__ == '__main__':
    {'evaluator': run_evaluator, 'sampler': run_sampler}[sys.argv[1]](Fixture)
