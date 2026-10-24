"""Two-label frozen sampler fixture with an explicit fresh-stage transfer."""

import copy
import numpy as np
from gammaboard_process import SampleBatch, run_sampler


class Sampler:
    def __init__(self, *, domain, evaluator_metadata=None, inference=False, seed=7):
        discrete_cardinalities, continuous_dims = domain.require_rectangular_shape()
        assert discrete_cardinalities == [2] and continuous_dims == 1
        self.inference = inference
        self.rng = np.random.default_rng(seed)
        self.p = np.array([0.3, 0.7])
        self.pending = []
        self.produced = 0

    def generate(self, max_samples):
        labels = self.rng.choice(2, max_samples, p=self.p)
        # Uniform conditional proposals deliberately produce misses.
        t = self.rng.random((max_samples, 1))
        if not self.inference:
            self.pending.append(max_samples)
        self.produced += max_samples
        return SampleBatch(
            labels[:, None],
            t,
            1 / self.p[labels],
            None if self.inference else max_samples,
        )

    def feedback(self, feedback):
        values = feedback.values
        assert len(values) == self.pending.pop(0)
        assert np.isfinite(values).all()

    def discrete_mass(self, subspaces):
        return [
            1.0 if not s else float(self.p[s[0]]) if set(s) == {0} else None
            for s in subspaces
        ]

    def snapshot(self):
        return {
            "rng": copy.deepcopy(self.rng.bit_generator.state),
            "p": self.p.tolist(),
            "pending": list(self.pending),
            "produced": self.produced,
            "inference": self.inference,
        }

    @classmethod
    def from_snapshot(cls, *, snapshot, init_args, **kwargs):
        result = cls(**kwargs, **init_args)
        result.rng.bit_generator.state = snapshot["rng"]
        result.p = np.array(snapshot["p"])
        result.pending = list(snapshot["pending"])
        result.produced = snapshot["produced"]
        result.inference = snapshot["inference"]
        return result

    @classmethod
    def from_stage(cls, *, source_state, source_args, init_args, **kwargs):
        assert not source_state["pending"]
        result = cls(**kwargs, **init_args)
        result.p = np.array(source_state["p"])
        assert result.inference and result.produced == 0
        return result


if __name__ == "__main__":
    run_sampler(Sampler)
