# Synthetic queue benchmark

This suite exercises normal workers, PostgreSQL queues, point serialization,
accumulators and training feedback. Only integrand/sampler work is synthetic.
No GammaLoop state or Symbolica license is needed. Build before benchmarking:

```sh
cargo build --release --no-default-features
python3 scripts/benchmark_queue.py run --binary target/release/gammaboard \
  --output /tmp/queue-baseline
```

Use Python 3.11+ and `psql` inside `nix develop`. `just benchmark-queue ...`
forwards the same arguments. Compilation is separate from benchmark time.

## Five-minute default

The default has **16 cases**: 1/4/16/64 evaluators × nominal compute capacity
1,000/2,000,000 samples/s × steady/bursty sampling. Each uses 0.5 seconds warm-up
and 4 seconds measurement, with one repetition and deterministic timing.
A normal sampler worker is additional to the evaluator count.

Both a baseline run and a paired A/B invocation have a **300-second wall-time
budget**, including deployment and cleanup. The runner reuses workers across
cases with the same fleet size, assigning them through the normal `run resume`
CLI. It pauses and removes each case before reassigning the workers. No special
worker capabilities are involved. Workloads that cannot fit are rejected; a
runtime deadline stops an unexpectedly slow run and retains partial results.
`completion.json` records the actual total duration and whether it met the budget.
An interrupted or incomplete run is not a complete baseline.

The benchmark targets 100 ms evaluation batches and records progress/performance
at 100 ms intervals, so short cases contain multiple observations. Per-point
costs remain `evaluators / nominal_rate`; the expensive 64-worker, 1,000/s case
still costs **64 ms per point**. The normal minimum batch size can consequently
make its batches longer than 100 ms. These explicit benchmark settings are shared
between A and B; this suite does not measure changes to the production batch-time
default. Short runs detect coarse regressions; repeat paired runs before claiming
small gains. One repetition provides no estimate of run-to-run variance.

## Fast generation, then a 0.5-second stall

`inference` generates points without artificial generation delay and never
updates. `training_burst` also generates without artificial delay, but repeatedly:

1. Produces a finite training window as quickly as the queue allows.
2. Waits for all feedback from that window.
3. Spends **0.5 seconds** updating, producing no points during the update.
4. Starts producing the next window immediately afterward.

The window is `max(64, min(100000, ceil(nominal_rate * 0.25)))` points. The cap
keeps update cycles frequent enough for the short test. Ingestion has no artificial
delay. Feedback waiting is additional to the 0.5-second update. Small windows may
not fill all 64 workers because of minimum batch sizes; the recorded utilization
and queue trace expose that limitation. Every burst case must observe at least
two completed updates during measurement or the benchmark fails explicitly.

Timing noise defaults to zero for short comparisons. `--noise 0.1` enables the
shared Gaussian model with sigma equal to 10% of each configured mean component.

## A/B and focused cases

```sh
python3 scripts/benchmark_queue.py ab --baseline /path/to/A/gammaboard \
  --candidate /path/to/B/gammaboard --output /tmp/queue-ab

python3 scripts/benchmark_queue.py run --binary target/release/gammaboard \
  --evaluators 4 --rates 1000 --regimes training_burst \
  --duration 20 --repetitions 3 --output /tmp/burst-repeat
```

Corresponding A/B cases use identical seeds and workloads. Repetitions reverse
A/B order. Binary copies are frozen in the output directory before measurement.
Filters and durations must fit the same five-minute budget. `--smoke` selects four
cases spanning all fleet sizes. Additional supported regimes are `batch_overhead`
(250 ms per batch), `sampler_limited` (generation capacity half compute capacity),
`training_large` (8 seconds nominal work, 10 ms update), and `training_small`
(0.25 seconds nominal work, 500 ms update, without the burst window cap).

Generate ordinary TOML cards for manual or external-worker deployments:

```sh
python3 scripts/benchmark_queue.py generate --output /tmp/queue-cards
```

The automatic runner manages only its own isolated local deployment. It refuses
occupied ports (default offset 30), uses a separate resource root with 512 allowed
PostgreSQL connections, and retains database files and logs after shutdown.
Use `--port-offset` to choose another isolated range. Avoid competing workloads
when comparing results.

## Results and recovery

- `manifest.json`: workloads, settings, host, source tree state, suite/runner hashes
  and executable hashes. The working-tree commit does not prove provenance of an
  arbitrary prebuilt executable.
- `results.jsonl`: completed-case journal with throughput, queue occupancy,
  utilization, update counts, actual mean update duration and engine diagnostics.
- Per-case TOML, raw JSONL and CSV: 100 ms observations of produced/completed
  sample rates, update counts and pending/claimed batches. These show the burst,
  stall and recovery instead of averaging them into a single sampler delay.
- `summary.md`/`summary.json`: rates and stall measurements; `comparison.json`
  contains paired B/A ratios. `completion.json` includes deployment/cleanup time.

Utilization uses the dashboard's active-time rolling window and includes startup
history. Throughput uses persisted completed-sample deltas. Training barrier
counters follow sampler snapshot timestamps, so `barrier_observation_seconds`
records their actual observation span separately. Compute capacities exclude
batch overhead, queue costs and training barriers; they are not promised rates.

Output directories must be new. To continue an interrupted run, repeat the exact
command with `--resume`; completed cases are skipped after verifying settings,
host and saved binaries. Incomplete attempts keep their raw observations and
restart in a fresh deployment directory. Finish shutting down any interrupted
deployment before resuming. Each invocation retains its five-minute budget.

A recorded run and its repeatability checks are in the
[compact baseline report](baselines/9ee7828.md).

## Shared synthetic engines

The user-facing `unit` evaluator accepts `timing`. `naive_monte_carlo` accepts
`generation_timing`, `ingest_timing`, `update_timing`, `seed` and
`training_window_samples` (zero means inference). Timing tables specify
`per_sample_seconds`, `overhead_seconds`, `sigma_per_sample_seconds`,
`sigma_overhead_seconds` and `seed`:

```
max(0, N * (per_sample_seconds + Normal(0, sigma_per_sample_seconds))
       + overhead_seconds + Normal(0, sigma_overhead_seconds))
```

One Gaussian pair is drawn per operation. Per-sample noise is correlated across
that batch; negative durations are clipped and counted. Requested and actual
waits are recorded separately. These waits are additional to actual point work.
Evaluator draws depend on batch size and endpoint coordinates, independently of
worker assignment/retries. Sampler timing draws use sample/update counters;
point RNG state is checkpointed independently. Changing batch boundaries changes
noise realizations even with paired seeds.

Training snapshots preserve pending feedback, update counts and RNG state.
A task ending partway through a window does not perform an incomplete update.
Synthetic waits model service time, not CPU/cache/NUMA or remote network effects.

```sh
python3 -m unittest discover -s benchmarks/queue -p 'test_*.py'
cargo test synthetic
# With an isolated test database:
cargo test --test full_stack_cli full_stack_synthetic_training_windows_and_inference -- --ignored
```

The old millisecond-only evaluator delay and one-shot sampler training settings
are replaced by the timing tables and repeating windows. Ordinary CI tests
correctness and timing-model behavior, not tight throughput thresholds.
