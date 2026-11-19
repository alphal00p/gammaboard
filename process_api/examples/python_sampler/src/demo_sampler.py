"""Uniform NumPy sampler illustrating the process contract, without adaptation."""

from collections import deque
import math

import numpy as np
from gammaboard_process import GenerationStatus, SampleBatch, Sampler


class UniformSampler(Sampler):
    def __init__(
        self, *, domain, evaluator_metadata=None, seed=0,
        training_samples=0, feedback_window=32,
    ):
        self.cards, self.continuous_dims = domain.require_rectangular_shape()
        if training_samples < 0 or feedback_window < 1:
            raise ValueError(
                "training_samples must be nonnegative; feedback_window must be positive"
            )
        self.training_samples = training_samples
        self.feedback_window = feedback_window
        self.volume = math.prod(self.cards)
        # MT19937 state consists of small integers, so snapshots are portable JSON.
        self.rng = np.random.Generator(np.random.MT19937(seed))
        self.generated = 0
        self.returned = 0
        self.pending = deque()

    def generate(self, max_samples):
        if max_samples <= 0:
            raise ValueError("max_samples must be positive")
        remaining = None
        if self.training_samples:
            ungenerated = self.training_samples - self.generated
            if ungenerated == 0:
                return (
                    GenerationStatus.WAITING if self.pending else GenerationStatus.FINISHED
                )
            remaining = min(ungenerated, self.feedback_window - sum(self.pending))
            if remaining == 0:
                return GenerationStatus.WAITING
        n = min(max_samples, remaining) if remaining is not None else max_samples
        discrete = np.empty((n, len(self.cards)), dtype=np.int64)
        for axis, cardinality in enumerate(self.cards):
            discrete[:, axis] = self.rng.integers(cardinality, size=n)
        continuous = self.rng.random((n, self.continuous_dims))
        self.generated += n
        if remaining is not None:
            self.pending.append(n)
        return SampleBatch(
            discrete, continuous, np.full(n, float(self.volume)),
            training_remaining=remaining,
        )

    def feedback(self, feedback):
        if not self.pending or len(feedback.values) != self.pending[0]:
            raise ValueError("feedback must match the oldest outstanding draw")
        self.returned += self.pending.popleft()
        # A learning sampler would update its proposal using feedback.values here.
        # GammaBoard has already applied the integration weight and stratum mask.

    def snapshot(self):
        rng = self.rng.bit_generator.state["state"]
        return {
            "rng": {"key": rng["key"].tolist(), "pos": int(rng["pos"])},
            "generated": self.generated,
            "returned": self.returned,
            "pending": list(self.pending),
        }

    @classmethod
    def from_snapshot(cls, *, snapshot, domain, init_args, evaluator_metadata=None):
        sampler = cls(domain=domain, evaluator_metadata=evaluator_metadata, **init_args)
        sampler.rng.bit_generator.state = {
            "bit_generator": "MT19937",
            "state": {
                "key": np.asarray(snapshot["rng"]["key"], dtype=np.uint32),
                "pos": snapshot["rng"]["pos"],
            },
        }
        sampler.generated = snapshot["generated"]
        sampler.returned = snapshot["returned"]
        sampler.pending = deque(snapshot["pending"])
        return sampler

    @classmethod
    def from_stage(
        cls, *, source_state, source_args, init_args, domain, evaluator_metadata=None,
    ):
        if init_args.get("training_samples", 0):
            raise ValueError("the destination stage must use training_samples=0")
        # Uniform sampling has no learned proposal to transfer. Start a fresh
        # stream and counters; exact checkpoint recovery uses from_snapshot.
        return cls(domain=domain, evaluator_metadata=evaluator_metadata, **init_args)

    def pdf(self, xs_discrete, xs_continuous):
        inside = np.all((xs_continuous >= 0) & (xs_continuous <= 1), axis=1)
        for axis, cardinality in enumerate(self.cards):
            inside &= (xs_discrete[:, axis] >= 0) & (xs_discrete[:, axis] < cardinality)
        return np.where(inside, 1.0 / self.volume, 0.0)

    def discrete_mass(self, subspaces):
        def mass(query):
            if any(
                d < 0 or d >= len(self.cards) or v < 0 or v >= self.cards[d]
                for d, v in query.items()
            ):
                return 0.0
            return math.prod(1.0 / self.cards[d] for d in query)

        return [mass(query) for query in subspaces]

    def diagnostics(self):
        return {
            "generated_samples": self.generated,
            "feedback_samples": self.returned,
            "pending_draws": len(self.pending),
        }
