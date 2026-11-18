"""Small real SDK workers and negative controls for the standalone checker."""

from __future__ import annotations

import copy
import json
import math
import os
import time
import uuid
from pathlib import Path

import numpy as np
from gammaboard_process import GenerationStatus, SampleBatch, Sampler
from gammaboard_process.domain import RectangularDomain


class UniformSampler(Sampler):
    def __init__(
        self,
        *,
        domain,
        evaluator_metadata,
        seed=13,
        training=False,
        window=10,
        limit=None,
        save_path=None,
        **kwargs,
    ):
        self.domain = domain
        self.metadata = evaluator_metadata
        self.rng = np.random.RandomState(seed)
        self.training = training
        self.window = window
        self.limit = limit
        self.save_path = save_path
        self.generated = 0
        self.returned = 0
        self.pending = []
        self.committed = set()
        self.extra = kwargs

    def point(self, node):
        if isinstance(node, RectangularDomain):
            discrete = [int(self.rng.randint(n)) for n in node.discrete_cardinalities]
            return (
                discrete,
                self.rng.random_sample(node.continuous_dims),
                math.prod(node.discrete_cardinalities),
            )
        branch = node.branches[int(self.rng.randint(len(node.branches)))]
        discrete, continuous, weight = self.point(branch.domain)
        return [branch.index, *discrete], continuous, weight * len(node.branches)

    def generate(self, max_samples):
        if self.limit is not None and self.generated >= self.limit:
            return GenerationStatus.FINISHED
        remaining = (
            self.window - (self.generated - self.returned) if self.training else None
        )
        if remaining == 0:
            return GenerationStatus.WAITING
        count = min(max_samples, remaining or max_samples)
        if self.limit is not None:
            count = min(count, self.limit - self.generated)
        points = [self.point(self.domain) for _ in range(count)]
        discrete, continuous, weights = zip(*points)
        self.generated += count
        if self.training:
            self.pending.append(count)
        widths = self.domain.fixed_widths()
        discrete = (
            np.asarray(discrete, dtype=np.int64).reshape(count, widths[0])
            if widths[0] is not None
            else tuple(np.asarray(d, dtype=np.int64) for d in discrete)
        )
        continuous = (
            np.asarray(continuous).reshape(count, widths[1])
            if widths[1] is not None
            else tuple(np.asarray(c) for c in continuous)
        )
        return SampleBatch(
            discrete,
            continuous,
            np.asarray(weights, dtype=float),
            training_remaining=remaining,
        )

    def feedback(self, feedback):
        assert self.pending, "unexpected feedback"
        count = self.pending.pop(0)
        assert len(feedback.values) == count, "feedback draw size/order mismatch"
        assert np.isfinite(feedback.values).all()
        self.returned += count

    def state(self):
        rng = self.rng.get_state()
        return {
            "rng": [rng[0], rng[1].tolist(), rng[2], rng[3], rng[4]],
            "generated": self.generated,
            "returned": self.returned,
            "pending": list(self.pending),
            "training": self.training,
        }

    def snapshot(self):
        state = self.state()
        if self.save_path:
            path = Path(self.save_path)
            path.mkdir(parents=True, exist_ok=True)
            file = path / (uuid.uuid4().hex + ".json")
            file.write_text(json.dumps(state))
            return {"path": str(file)}
        return state

    @classmethod
    def from_snapshot(cls, *, snapshot, domain, init_args, evaluator_metadata):
        state = (
            json.loads(Path(snapshot["path"]).read_text())
            if "path" in snapshot
            else snapshot
        )
        result = cls(domain=domain, evaluator_metadata=evaluator_metadata, **init_args)
        rng = state["rng"]
        result.rng.set_state((rng[0], np.asarray(rng[1], dtype=np.uint32), *rng[2:]))
        result.generated = state["generated"]
        result.returned = state["returned"]
        result.pending = list(state["pending"])
        result.training = state["training"]
        return result

    @classmethod
    def from_stage(
        cls, *, source_state, source_args, init_args, domain, evaluator_metadata
    ):
        assert isinstance(source_state, dict)
        assert isinstance(source_args, dict)
        result = cls(domain=domain, evaluator_metadata=evaluator_metadata, **init_args)
        result.training = False
        return result

    def checkpoint_committed(self, *, snapshot):
        self.committed.add(json.dumps(snapshot, sort_keys=True))

    def pdf(self, xs_discrete, xs_continuous):
        values = []
        for row in xs_discrete:
            node = self.domain
            probability = 1.0
            depth = 0
            while not isinstance(node, RectangularDomain):
                probability /= len(node.branches)
                node = next(b.domain for b in node.branches if b.index == row[depth])
                depth += 1
            probability /= math.prod(node.discrete_cardinalities)
            values.append(probability)
        return np.asarray(values)

    def discrete_mass(self, subspaces):
        if not isinstance(self.domain, RectangularDomain):
            return None
        return [
            math.prod(1 / self.domain.discrete_cardinalities[d] for d in query)
            for query in subspaces
        ]

    def diagnostics(self):
        return {"generated": self.generated, "returned": self.returned}

    def persisted_output(self):
        return None


class InferenceOnly(UniformSampler):
    checkpoint_committed = Sampler.checkpoint_committed
    from_stage = Sampler.from_stage
    pdf = Sampler.pdf
    discrete_mass = Sampler.discrete_mass
    diagnostics = Sampler.diagnostics
    persisted_output = Sampler.persisted_output


class NoSnapshot(UniformSampler):
    def snapshot(self):
        return None


class LargeIntegerSnapshot(UniformSampler):
    def snapshot(self):
        return {"rng_integer": 2**100 + 3}


class BrokenRestore(UniformSampler):
    @classmethod
    def from_snapshot(cls, *, snapshot, domain, init_args, evaluator_metadata):
        return cls(
            domain=domain,
            evaluator_metadata=evaluator_metadata,
            **{**init_args, "seed": 500},
        )


class LostFeedback(UniformSampler):
    def snapshot(self):
        state = copy.deepcopy(super().snapshot())
        state["pending"] = []
        return state


class OverwriteSnapshot(UniformSampler):
    def snapshot(self):
        path = Path(self.save_path)
        path.mkdir(parents=True, exist_ok=True)
        file = path / "overwritten.json"
        file.write_text(json.dumps(self.state()))
        return {"path": str(file)}


class NonIdempotentNotification(UniformSampler):
    def checkpoint_committed(self, *, snapshot):
        key = json.dumps(snapshot, sort_keys=True)
        if key in self.committed:
            raise ValueError("duplicate notification")
        self.committed.add(key)


class NegativePdf(UniformSampler):
    def pdf(self, xs_discrete, xs_continuous):
        return [-1.0] * len(xs_discrete)


class PartialPdf(UniformSampler):
    def pdf(self, xs_discrete, xs_continuous):
        values = super().pdf(xs_discrete, xs_continuous).tolist()
        values[0] = None
        return values


class BadMass(UniformSampler):
    def discrete_mass(self, subspaces):
        return [0.5] * len(subspaces)


class NoMass(UniformSampler):
    def discrete_mass(self, subspaces):
        return None


class InvalidWeights(UniformSampler):
    def generate(self, max_samples):
        batch = super().generate(max_samples)
        batch.weights[:] = -1
        return batch


class OversizedBatch(UniformSampler):
    def generate(self, max_samples):
        return super().generate(max_samples + 1)


class Deadlock(UniformSampler):
    def generate(self, max_samples):
        return GenerationStatus.WAITING


class HungSampler(UniformSampler):
    def generate(self, max_samples):
        if self.save_path:
            Path(self.save_path).write_text(str(os.getpid()))
        time.sleep(60)


class OnlyContinuous(UniformSampler):
    def __init__(self, **kwargs):
        domain = kwargs["domain"]
        if not isinstance(domain, RectangularDomain) or domain.discrete_cardinalities:
            raise ValueError("only supports a continuous domain")
        super().__init__(**kwargs)


class BrokenStage(UniformSampler):
    @classmethod
    def from_stage(cls, **kwargs):
        raise ValueError("broken stage initialization")


def main():
    """A console-script target using the ordinary SDK, without probe RPCs."""
    import argparse

    from gammaboard_process import run_sampler
    from gammaboard_process.log import install_print_redirect
    from gammaboard_process.runners import _SamplerWorker

    parser = argparse.ArgumentParser()
    parser.add_argument("sampler")
    parser.add_argument("--legacy", action="store_true")
    options = parser.parse_args()
    assert os.environ["GAMMABOARD_PROCESS_RUNTIME"] == "command"
    assert os.environ["GAMMABOARD_PROCESS_WORKER"] == "sampler"
    target = globals()[options.sampler]
    if not options.legacy:
        run_sampler(target)
        return

    class LegacyWorker(_SamplerWorker):
        def handle(self, method, params, req_binary=b""):
            if method == "checkpoint_committed":
                raise ValueError("legacy worker received an unadvertised notification")
            result = super().handle(method, params, req_binary)
            if method == "initialize":
                result.pop("checkpoint_committed", None)
            return result

    install_print_redirect()
    LegacyWorker(target).run()


if __name__ == "__main__":
    main()
