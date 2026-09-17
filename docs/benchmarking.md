# Measuring overhead and scaling

Use the CLI for runtime data and execution. The Python driver defines experiments,
invokes the CLI, saves raw JSON, and plots it; it does not query PostgreSQL or parse
dashboard panels. The older `benchmark-queue` remains a short sleep-based queue
stress test, and `benchmark-campaign` remains a storage-growth soak test.

## Inspection

```sh
gammaboard --json run performance RUN
gammaboard --json run wait RUN --until ready --evaluators 4 --timeout 60s
gammaboard --json run performance RUN --duration 30s --interval 1s
gammaboard --json run performance RUN --since 2026-09-17T00:00:00Z --until 2026-09-18T00:00:00Z
gammaboard --json run wait RUN --until idle
```

`ready` requires a live sampler, the requested evaluator count, settled assignments,
and recent telemetry from initialized runtimes. Warmup is a separate step. `idle`
means no live desired or active assignments to this run; `completed` waits for all
its tasks to finish and fails if any failed. These commands concern the selected
run, not recursively its children.

The authenticated `GET /api/runs/:id/metrics` endpoint returns the same snapshot
as the CLI. The existing `/performance` dashboard endpoint remains unchanged.

JSON has `schema_version = 1`. Names ending in `_seconds` are seconds; rates are
samples per second. `completed_samples` is accepted progress, not attempted work.
`allocated_core_seconds` is allocated worker time from heartbeat accounting, not
OS CPU consumption, and excludes the database/server. OS CPU consumption is not
currently measured. Evaluator cumulative timings cover successfully submitted
batches in one runner epoch; they are wall durations, not CPU durations. Fetch
wait totals cover successful batches, not all empty queue polls. Use the rolling
starvation diagnostic for the latter.

A snapshot is one consistent database read, but worker publications are
asynchronous. Every telemetry row includes its publication time. Intervals use
counter differences and a monotonic observation clock. They return all sampled
snapshots plus evaluator counter deltas with their own publication boundaries.
Do not sum asynchronous phase durations into elapsed time, average rolling means
into interval totals, or interpret publication boundaries as synchronized worker
barriers. The default maximum telemetry age is 10 seconds (`--max-age`).

Intervals containing observed task/assignment changes, incarnation changes,
missing/stale telemetry, or counter regressions have `valid = false`, explicit
`issues`, and a null throughput. Assignment-change counts describe sampled
transitions; they are not a complete event audit. Evaluator and sampler epochs
also detect restarts between polls once the new incarnation publishes telemetry. Short publication intervals and
longer trials reduce boundary error; old binaries without epochs are marked as
missing coverage. Historical export has a default 1,000-row limit and an explicit
`truncated` flag, with a maximum of 10,000 rows. Narrow the time range if truncated.

## Direct baseline and fixed CPU work

An evaluator card contains the ordinary evaluator definition:

```toml
[evaluator]
kind = "unit"
continuous_dims = 6
cpu_iterations_per_sample = 10000
```

```sh
gammaboard --json benchmark calibrate --eval-us 100
gammaboard --json benchmark evaluator evaluator.toml --workers 4 --batch-size 256 --warmup 2s --duration 5s
gammaboard --json benchmark evaluator evaluator.toml --workers 4 --batch-size 256 --samples 100000
```

Calibration returns a fixed iteration count. Reuse it across all worker counts;
never recalibrate work as concurrency changes. Unlike sleep or spinning until a
wall-clock deadline, fixed arithmetic work exposes contention. The unit evaluator
still returns one, so scientific results remain checkable. Zero iterations
preserves the existing evaluator's behavior.

The direct baseline uses production factories and code for the evaluator,
uniform sample generation, identity materialization, and scalar accumulation.
Each worker generates and accumulates locally. This is a simple local parallel
baseline, not a simulation of the central sampler. It does not implement adaptive
training or arbitrary controller run cards. Evaluators must support uniform-domain
sampling and a scalar accumulator. Initialization and total command-internal time
are reported separately. `--samples` executes an exact finite sample count;
without warmup its total includes initialization, but excludes process launch.

`sampler_aggregator_runner_params.queue.fixed_batch_size` disables automatic
batch adaptation. It is clamped to the existing minimum of 16 and to
`max_batch_size`; remaining sample budgets and training boundaries may shorten a
batch. Omitting it preserves automatic adaptation.

## Running the suite

Use an optimized, prebuilt binary inside the development shell:

```sh
just benchmark run resources/templates/benchmarks/smoke.toml --binary target/dev-optim/gammaboard --output results/smoke
just benchmark run resources/templates/benchmarks/scaling.toml --binary target/release/gammaboard --output results/scaling
just benchmark plot results/scaling
just benchmark compare results/before results/after
```

Python 3.11+ is needed for running. Plotting additionally needs matplotlib; it is
separate so results can be plotted on another machine. No Python database driver
or `psql` is required by this suite.

The default scaling suite has 48 cases: four evaluation costs, four worker
counts, one batch size, and three repetitions, with randomized order. Each case
has direct serial, direct parallel, and GammaBoard measurements. All use the same
fixed work and batch size. The presets explicitly use `min_tick_time_ms = 0`
and `telemetry_interval_ms = 250`: this measures the pipeline without the normal
10ms runner tick floor. Set `min_tick_time_ms = 10` for the default polling policy;
vary the telemetry interval to investigate publication overhead. These settings
are saved in run cards and the plots identify the tick floor and core budget.
Calibration targets are labels, not claims of exact
service time; raw measured calibration and direct throughput are saved.

The suite rejects plans estimated to exceed its budget, enforces a deadline on
CLI calls, and reserves 30 seconds for deployment cleanup. Its maximum budget is
30 minutes, excluding compilation and plotting. It uses a private deployment,
checks ports first (`--port-offset`), and uses GammaBoard's own shutdown protocol.
Raw results survive successful cleanup; failures retain deployment diagnostics.
Invalid cases are saved with their issues and make the driver exit unsuccessfully.
Use `--calibration previous/calibration.json` to keep work identical across code
revisions. `compare` rejects overlapping cases with different work counts.

On Linux the runner selects relatively idle, distinct physical cores from its
existing affinity. All descendants, including PostgreSQL and the server, inherit
the same restricted set. It caps usage to eight physical cores and at most a
quarter of available physical cores, lowers scheduling priority, and limits
common thread pools to one thread per worker. This limits consumption; it does
not reserve cores from other users. Background load is part of the uncertainty.
The entire local comparison shares that core budget; eight evaluator workers
therefore compete with sampler/database work within the same eight cores.

Results contain the suite, calibration, exact cards, binary hash, CPU affinity,
host metadata, raw direct measurements, and complete GammaBoard intervals.
`results.jsonl` is appended after every case so interruption retains completed
work. Plot commands operate entirely offline and write PNG and SVG files.

## Interpretation and scope

- `speedup` is GammaBoard throughput divided by direct serial throughput.
- `retained_efficiency` is GammaBoard throughput divided by direct parallel
  throughput with the same evaluator count and enclosing CPU budget.
- Error bars show the range of independent trial results around their median,
  not confidence intervals computed from correlated telemetry snapshots.
- A missing/invalid case is not zero throughput.

The first suite measures warm, fixed-batch CPU inference. It does not establish
cold-job crossover sizes, GPU efficiency, multi-host networking, or time to a
physics uncertainty target. Those should be additional experiment types using
the same measurement contract. Sample-level synthetic costs alone do not predict
memory-bound or vectorized physics behavior. Keep batch size, calibrated work,
resource budget, and telemetry frequency visible when comparing results.
