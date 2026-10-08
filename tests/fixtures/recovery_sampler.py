"""Exact recovery oracle using the real process SDK, with self-contained snapshots.

Every point has an identifiable dyadic value and a non-unit importance weight.
Feedback must match the oldest draw exactly. No benchmark or production switches.
"""
from collections import deque
from pathlib import Path
import sys

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "process_api/python/src"))
from gammaboard_process import GenerationStatus, SampleBatch, run_sampler


class RecoverySampler:
    def __init__(self, *, continuous_dims, discrete_cardinalities,
                 training=False, window=73, offset=0, **kwargs):
        assert continuous_dims == 1 and not discrete_cardinalities
        self.training = training
        self.window = window
        self.offset = offset
        self.produced = 0
        self.accepted = 0
        self.pending = deque()
        self.feedback_calls = 0

    def generate(self, max_samples):
        if self.training and self.produced % self.window == 0 and self.accepted < self.produced:
            return GenerationStatus.WAITING
        remaining = self.window - self.produced % self.window
        size = min(max_samples, remaining) if self.training else max_samples
        start = self.produced
        self.produced += size
        if self.training:
            self.pending.append((start, size))
        x = (self.offset + np.arange(start, start + size) + 1) / 65536.0
        return SampleBatch(np.empty((size, 0), dtype=np.int64), x[:, None],
                           np.full(size, 2.0),
                           training_remaining=remaining if self.training else None)

    def feedback(self, values):
        assert self.training and self.pending, "unsolicited/duplicate feedback"
        start, size = self.pending[0]
        expected = (self.offset + np.arange(start, start + size) + 1) / 32768.0
        assert start == self.accepted, "feedback order changed"
        assert np.array_equal(values, expected), "feedback values/weights/draw boundaries changed"
        self.pending.popleft()
        self.accepted += size
        self.feedback_calls += 1

    def snapshot(self):
        return dict(produced=self.produced, accepted=self.accepted,
                    pending=list(self.pending), feedback_calls=self.feedback_calls)

    @classmethod
    def from_snapshot(cls, *, snapshot, init_args, **kwargs):
        sampler = cls(**kwargs, **init_args)
        sampler.produced = snapshot["produced"]
        sampler.accepted = snapshot["accepted"]
        sampler.pending = deque(map(tuple, snapshot["pending"]))
        sampler.feedback_calls = snapshot["feedback_calls"]
        assert sampler.accepted <= sampler.produced
        if sampler.training:
            cursor = sampler.accepted
            for start, size in sampler.pending:
                assert start == cursor and size > 0, "incoherent sampler snapshot"
                cursor += size
            assert cursor == sampler.produced, "snapshot lost pending feedback"
        return sampler

    def get_diagnostics(self):
        return self.snapshot()


if __name__ == "__main__":
    run_sampler(RecoverySampler)
