from __future__ import annotations

import base64
from typing import Any

from symbolica import NumericalIntegrator, Sample


class SampleBatch:
    def __init__(self, xs_discrete, xs_continuous, weights) -> None:
        self.xs_discrete = xs_discrete
        self.xs_continuous = xs_continuous
        self.weights = weights


def build_integrator(
    discrete_cardinalities: list[int],
    continuous_dims: int,
    bins: int,
    samples_for_update: int,
) -> NumericalIntegrator:
    if not discrete_cardinalities:
        return NumericalIntegrator.continuous(continuous_dims, bins, samples_for_update)

    first, *rest = discrete_cardinalities
    return NumericalIntegrator.discrete(
        [
            build_integrator(rest, continuous_dims, bins, samples_for_update)
            for _ in range(first)
        ]
    )


def sample_weight(sample: Sample) -> float:
    if not sample.weights:
        raise ValueError("symbolica sample did not expose weights")
    return float(sample.weights[0])


class SymbolicaHavanaSampler:
    def __init__(
        self,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        seed: int = 0,
        bins: int = 64,
        samples_for_update: int = 10_240,
        stop_training_after_n_samples: int = 10_240,
        initial_training_rate: float = 0.1,
        final_training_rate: float = 0.1,
        integrator: NumericalIntegrator | None = None,
    ) -> None:
        self.discrete_cardinalities = [int(value) for value in discrete_cardinalities]
        self.continuous_dims = int(continuous_dims)
        self.seed = int(seed)
        self.bins = int(bins)
        self.samples_for_update = int(samples_for_update)
        self.stop_training_after_n_samples = int(stop_training_after_n_samples)
        self.initial_training_rate = float(initial_training_rate)
        self.final_training_rate = float(final_training_rate)

        if self.bins <= 0 or self.samples_for_update <= 0:
            raise ValueError("bins and samples_for_update must be > 0")
        if self.stop_training_after_n_samples <= 0:
            raise ValueError("stop_training_after_n_samples must be > 0")
        if self.continuous_dims < 0:
            raise ValueError("continuous_dims must be >= 0")
        if any(cardinality <= 0 for cardinality in self.discrete_cardinalities):
            raise ValueError("discrete_cardinalities must all be > 0")
        if self.initial_training_rate < 0.0 or self.final_training_rate < 0.0:
            raise ValueError("training rates must be >= 0")

        self.integrator = integrator or build_integrator(
            self.discrete_cardinalities,
            self.continuous_dims,
            self.bins,
            self.samples_for_update,
        )
        self.pending_samples: list[list[Sample]] = []
        self.batches_produced = 0
        self.samples_produced = 0
        self.batches_ingested = 0
        self.samples_ingested = 0

    @classmethod
    def from_config(
        cls,
        *,
        discrete_cardinalities: list[int],
        continuous_dims: int,
        init_args: dict[str, Any],
    ) -> "SymbolicaHavanaSampler":
        return cls(
            discrete_cardinalities=discrete_cardinalities,
            continuous_dims=continuous_dims,
            **(init_args or {}),
        )

    @classmethod
    def from_snapshot(
        cls,
        *,
        snapshot: dict[str, Any],
        discrete_cardinalities: list[int],
        continuous_dims: int,
        init_args: dict[str, Any],
    ) -> "SymbolicaHavanaSampler":
        args = init_args or {}
        grid_b64 = snapshot.get("grid_b64")
        if not isinstance(grid_b64, str):
            raise ValueError("snapshot missing grid_b64")

        sampler = cls(
            discrete_cardinalities=discrete_cardinalities,
            continuous_dims=continuous_dims,
            seed=int(snapshot.get("seed", args.get("seed", 0))),
            bins=int(snapshot.get("bins", args.get("bins", 64))),
            samples_for_update=int(
                snapshot.get("samples_for_update", args.get("samples_for_update", 10_240))
            ),
            stop_training_after_n_samples=int(
                snapshot.get(
                    "stop_training_after_n_samples",
                    args.get("stop_training_after_n_samples", 10_240),
                )
            ),
            initial_training_rate=float(
                snapshot.get("initial_training_rate", args.get("initial_training_rate", 0.1))
            ),
            final_training_rate=float(
                snapshot.get("final_training_rate", args.get("final_training_rate", 0.1))
            ),
            integrator=NumericalIntegrator.import_grid(base64.b64decode(grid_b64)),
        )
        sampler.batches_produced = int(snapshot.get("batches_produced", 0))
        sampler.samples_produced = int(snapshot.get("samples_produced", 0))
        sampler.batches_ingested = int(snapshot.get("batches_ingested", 0))
        sampler.samples_ingested = int(snapshot.get("samples_ingested", 0))
        return sampler

    def pending_training_sample_count(self) -> int:
        return sum(len(samples) for samples in self.pending_samples)

    def remaining_training_samples_to_produce(self) -> int:
        return max(
            0,
            self.stop_training_after_n_samples
            - self.samples_ingested
            - self.pending_training_sample_count(),
        )

    def training_window_samples_remaining(self) -> int:
        remaining = self.remaining_training_samples_to_produce()
        if remaining == 0:
            return 0
        current_window_end = (
            (self.samples_ingested // self.samples_for_update) + 1
        ) * self.samples_for_update
        inflight_or_ingested = self.samples_ingested + self.pending_training_sample_count()
        return min(remaining, max(0, current_window_end - inflight_or_ingested))

    def current_training_rate(self) -> float:
        progress = min(self.samples_ingested, self.stop_training_after_n_samples) / float(
            self.stop_training_after_n_samples
        )
        if self.initial_training_rate <= 0.0 or self.final_training_rate <= 0.0:
            return self.initial_training_rate + (
                self.final_training_rate - self.initial_training_rate
            ) * progress
        return self.initial_training_rate * (
            self.final_training_rate / self.initial_training_rate
        ) ** progress

    def training_samples_remaining(self) -> int | None:
        remaining = self.remaining_training_samples_to_produce()
        return remaining if remaining else None

    def sample_plan(self) -> dict[str, Any]:
        nr_samples = self.training_window_samples_remaining()
        if nr_samples == 0:
            return {"kind": "pause"}
        return {"kind": "produce", "nr_samples": nr_samples}

    def produce_latent_batch(self, nr_samples: int) -> SampleBatch:
        rng = NumericalIntegrator.rng(self.seed, self.batches_produced)
        samples = list(self.integrator.sample(nr_samples, rng))
        self.pending_samples.append(samples)
        self.batches_produced += 1
        self.samples_produced += nr_samples
        return SampleBatch(
            xs_discrete=[list(map(int, sample.d)) for sample in samples],
            xs_continuous=[list(map(float, sample.c)) for sample in samples],
            weights=[sample_weight(sample) for sample in samples],
        )

    def ingest_training_values(self, training_values: Any) -> None:
        if not self.pending_samples:
            raise ValueError("received training values with no pending training batch")

        samples = self.pending_samples.pop(0)
        values = [float(value) for value in training_values]
        if len(values) != len(samples):
            raise ValueError("training value count does not match the pending sample count")

        before = self.samples_ingested
        train_len = min(self.stop_training_after_n_samples - self.samples_ingested, len(values))
        if train_len > 0:
            raw_values = [
                values[index] / sample_weight(samples[index]) for index in range(train_len)
            ]
            self.integrator.add_training_samples(samples[:train_len], raw_values)

        self.batches_ingested += 1
        self.samples_ingested += train_len

        previous_window = before // self.samples_for_update
        current_window = self.samples_ingested // self.samples_for_update
        for _ in range(max(0, current_window - previous_window)):
            rate = self.current_training_rate()
            self.integrator.update(rate, rate)

    def snapshot(self) -> dict[str, Any]:
        return {
            "seed": self.seed,
            "bins": self.bins,
            "samples_for_update": self.samples_for_update,
            "stop_training_after_n_samples": self.stop_training_after_n_samples,
            "initial_training_rate": self.initial_training_rate,
            "final_training_rate": self.final_training_rate,
            "batches_produced": self.batches_produced,
            "samples_produced": self.samples_produced,
            "batches_ingested": self.batches_ingested,
            "samples_ingested": self.samples_ingested,
            "grid_b64": base64.b64encode(self.integrator.export_grid()).decode("ascii"),
        }

    def get_diagnostics(self) -> dict[str, Any]:
        _, _, chi_sq, _, _, processed = self.integrator.get_live_estimate()
        return {
            "chi_sq": chi_sq,
            "processed_samples": processed,
            "samples_ingested": self.samples_ingested,
            "pending_training_samples": self.pending_training_sample_count(),
            "training_window_samples_remaining": self.training_window_samples_remaining(),
            "training_rate": self.current_training_rate(),
        }
