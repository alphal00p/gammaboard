# Synthetic queue benchmark

This suite exercises the real GammaBoard sampler, evaluator runners, PostgreSQL
queues, serialization, accumulators and training barriers. Only engine work is
synthetic: a unit evaluator returns one and waits for a configured duration;
a seeded uniform sampler simulates generation, feedback ingestion and updates.
No GammaLoop state, Python sampler, GPU or Symbolica license is needed for these
workloads. Use `--no-default-features` to build without GammaLoop support.

## Run

Use Python 3.11+ and `psql` inside `nix develop`. Build before measuring:

```sh
cargo build --release --no-default-features
python3 scripts/benchmark_queue.py run --binary target/release/gammaboard \
  --smoke --output /tmp/queue-smoke
```

The smoke suite covers all four evaluator counts (1, 4, 16, 64), using short
windows for functional verification, not performance conclusions. A normal
worker is additionally allocated to the sampler. The full suite has 80 cases:

```sh
python3 scripts/benchmark_queue.py run --binary target/release/gammaboard \
  --output /tmp/queue-full
```

Filter without changing the suite file:

```sh
python3 scripts/benchmark_queue.py run --binary target/release/gammaboard \
  --evaluators 1 4 --rates 1000 --regimes inference training_small \
  --warmup 10 --duration 30 --repetitions 3 --output /tmp/queue-subset
```

`just benchmark-queue ...` forwards the same arguments. Output directories must
be new; logs and results are never silently overwritten.

The runner starts its own deployment at port offset 30, refuses occupied ports,
uses a separate resource directory and stops its workers/deployment afterward.
It removes only runs it created. PostgreSQL data and logs remain in the output
folder for inspection. Use `--port-offset` for another isolated port range.
The runtime retains normal worker connection pools and allows 512 PostgreSQL
connections to support 65 ordinary workers. Record these settings when comparing
with another deployment. Ensure no other substantial workload competes with the
benchmark if you want meaningful performance comparisons.

For cluster deployment or manual inspection, generate ordinary run cards:

```sh
python3 scripts/benchmark_queue.py generate --output /tmp/queue-cards
```

Create a card through `gammaboard run create`, launch its specified evaluator
count plus a sampler using your normal local or external worker launch workflow,
and assign them normally. The automatic runner currently manages local workers;
TOML generation works for external deployments as well.

## A/B

Both binaries must support the synthetic timing configuration. Build and save
immutable executables before starting. The runner copies the executables into the output directory before starting,
so subsequent builds cannot replace a binary midway through a measurement.
It performs no compilation:

```sh
python3 scripts/benchmark_queue.py ab --baseline /path/to/A/gammaboard \
  --candidate /path/to/B/gammaboard --output /tmp/queue-ab
```

Each repetition runs the matrix with A then B, reversing that order on alternate
repetitions. Corresponding cases use the same seeds, dimensions, noise parameters
and sample budgets. Defaults are three repetitions, 30 seconds warm-up and
120 seconds measurement per case. `--noise 0` provides deterministic timing
references. Use more repetitions to resolve small changes.

`manifest.json` records suite parameters, host, working-tree commit/dirty state,
and hashes of the actual binaries (the working-tree commit does not establish
the provenance of an arbitrary prebuilt binary). Every case preserves its TOML,
raw observations and deployment log. `results.jsonl` includes throughput, fraction
of the compute ceiling, samples per evaluator-second, actual mean batch size,
rolling utilization, peak queue occupancy, recorded training-barrier time and
final engine timing diagnostics. Barrier time measures production blocking from
window exhaustion through feedback collection and update completion; shutdown
downtime is excluded on resume. Its measurement boundaries follow periodic
performance snapshots; `barrier_observation_seconds` records their actual time
span, which can differ from the independent progress-measurement window.
`summary.md` provides a readable report. `summary.json` reports repetition means and standard deviations; `comparison.json`
contains paired B/A ratios and their variation. One repetition has no estimated
variation. Do not interpret small differences as wins without repeatability.

Task progress is sampled independently of dashboard rate estimates. Utilization
is the dashboard's rolling measurement, so a short warm-up retains some earlier
history. Warm-up and measurement use the same running task; the common sample
budget is deliberately above expected progress so no stop boundary determines
the reported rate. The runner fails if the task stops, loses workers, or makes no
measured progress. Samples and training windows completed during warm-up are not
reset.

## Workloads

`queue/suite.toml` specifies 1/4/16/64 evaluators, nominal aggregate compute
ceilings 1k/10k/100k/2M samples/s, and five regimes. Per-sample evaluator time is
`evaluators / rate`: the 64-worker 1k/s case costs 64 ms per sample; its 2M/s
counterpart costs 32 microseconds. The matrix normalizes aggregate compute
capacity while changing per-worker work. It is not a fixed-integrand strong
scaling experiment.

| Regime | Parameters beyond per-sample evaluator time |
| --- | --- |
| inference | 1 ms batch overhead; generation capacity 8 times compute ceiling |
| batch_overhead | 250 ms batch overhead; generation capacity 8 times compute ceiling |
| sampler_limited | generation capacity half compute ceiling; 1 ms evaluator batch overhead |
| training_large | 8 seconds of nominal aggregate work per window; 10 ms updates |
| training_small | 0.25 seconds of nominal aggregate work per window (minimum 64 points); 500 ms updates |

Training cases also simulate ingestion at 100 times the compute ceiling plus
0.1 ms per returned batch. Noise sigma defaults to 10% of each configured mean
component. Queue controls retain the deployed defaults, deliberately making
changes to those defaults part of A/B comparisons. Run cards record workloads;
the binary and source revision determine queue defaults.

Compute ceilings exclude batch overhead, sampler work, training barriers and
GammaBoard's own work. Two million samples/s is a target to stress overhead, not
a promised achievable rate. Real concrete point payloads and accumulator updates
are retained; there is no shortcut that bypasses data movement at high rates.

Waiting simulates service time without burning 64 cores. It does not simulate
CPU cache effects, memory bandwidth contention, NUMA or remote network latency.
Validate promising scheduler changes on real workloads/hardware afterward.

## Shared synthetic timing

The user-facing `unit` evaluator accepts `timing`; `naive_monte_carlo` accepts
`generation_timing`, `ingest_timing`, `update_timing`, `seed` and
`training_window_samples`. Zero window means inference. Defaults have no delays.

Each timing table contains floating-point `per_sample_seconds`,
`overhead_seconds`, `sigma_per_sample_seconds`, `sigma_overhead_seconds` and an
integer `seed`. The duration is:

```
max(0, N * (per_sample_seconds + Normal(0, sigma_per_sample_seconds))
       + overhead_seconds + Normal(0, sigma_overhead_seconds))
```

One Gaussian pair is drawn per operation. The per-sample noise is correlated
across that whole batch; its standard deviation grows as N, not sqrt(N).
Durations are additional simulated work, not a target including real point
processing. Negative draws are clipped, counted and therefore bias the mean
upward near zero. Invalid/nonfinite parameters and duration overflow fail clearly.
Requested and actual waits are reported separately in worker panels and metrics.

Evaluator noise is keyed by batch size and endpoint coordinates, independent of
worker assignment and retries. Identical-content batches have identical noise;
use a nonzero continuous dimension for representative noisy benchmarks. Sampler
points use a checkpointed RNG, independent of timing draws and batch boundaries.
Generation/ingestion keys use their sample counters and update keys use the update
index. Repartitioning batches changes batch-correlated noise realizations even
with paired seeds; use repetitions and zero-noise references rather than claiming
identical random work across different batch layouts.

Training windows forbid generating beyond the window until all feedback returns
and the update delay finishes. Snapshot restoration preserves counters, pending
feedback and sample RNG state. A task stopping partway through its final window
does not perform an incomplete update. A fresh inference task uses a sampler
configuration with window zero; inheriting a training sampler retains its window.

The previous `min_eval_time_per_sample_ms`, `training_delay_per_sample_ms` and
one-shot `training_target_samples` synthetic settings are replaced by these
fields. Migrate old synthetic cards/checkpoints; production sampler formats are
unchanged.

## Tests

```sh
python3 -m unittest discover -s benchmarks/queue -p 'test_*.py'
cargo test synthetic
# Uses the normal full-stack harness and an isolated test database:
cargo test --test full_stack_cli full_stack_synthetic_training_windows_and_inference -- --ignored
```

Timing unit tests inject a waiter instead of sleeping. Full-stack tests use small
real delays and assert progress, exact training update counts and diagnostics;
ordinary CI does not assert tight performance thresholds.

## Continuing an interrupted benchmark

The full default run takes at least ten hours (80 cases × 3 repetitions ×
150 seconds), plus worker startup and shutdown. Completed cases are appended to
`results.jsonl`; summary files are derived from that journal. Repeat the original
command with `--resume` and the same options to skip completed cases. The runner
checks workload settings, host, thread limits, runner/suite hashes and binary
hashes, and uses the saved executable copies. An incomplete case starts again in
a fresh deployment directory; its earlier raw observations remain available.
An occupied port still fails explicitly: finish shutting down an interrupted
benchmark deployment before resuming. Do not combine records from different
binaries or measurement settings to make a baseline.
