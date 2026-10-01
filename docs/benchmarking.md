# Benchmarking GammaBoard

Use `just benchmark` for all experiments. The default sparse frontier measures
coordination and transport with sleeping evaluators. Process-API measurements
isolate adapter overhead; CPU and I/O suites answer targeted questions.
[Current results](performance-development.md) summarize the latest measurements.
[Sampling](sampling.md) defines generation, evaluator batches and feedback.

## Choose a measurement

| Question | Command | Scope |
| --- | --- | --- |
| Capability and scaling | `frontier` | Three data paths, four delays, 1–512 evaluator processes |
| External process overhead | `process` | Production Rust adapters and Python SDK, without the database |
| Insert/bundle/storage tuning | `io` | Repeated fixed-configuration comparisons |
| Fixed CPU-work efficiency | `run PRESET` | Pipeline versus direct serial/parallel evaluation |
| An existing run | `gammaboard run performance` | Current deployment and workload |

Run from the development shell with Python 3.11+ and an optimized binary.
Plotting also needs matplotlib; process tests need a Python interpreter with
NumPy. Compilation and plotting are outside suite budgets.

```sh
cargo build --locked --profile dev-optim --bin gammaboard
just benchmark plan resources/templates/benchmarks/frontier.toml
just benchmark frontier --binary target/dev-optim/gammaboard --output results/frontier
just benchmark plot results/frontier
just benchmark summary results/frontier
```

Outputs must be new directories. Deployment suites use private resources, check
ports, preserve diagnostics and shut down their own workers and database.
Use `--port-offset` to avoid other instances. CPU affinity limits placement;
it does not reserve cores on a shared host.

## Default sparse frontier

The [preset](../resources/templates/benchmarks/frontier.toml) defines **87 fresh-run
measurements**. The registered pool remains at 512 processes; each point selects
its active evaluators.

| Mode | Input | Result |
| --- | --- | --- |
| `rng` | Compact RNG checkpoints from a frozen uniform Havana grid | Compact accumulation |
| `materialized` | Six-dimensional materialized samples | Compact accumulation |
| `training` | Six-dimensional materialized samples | Accumulation and per-sample feedback |

All modes evaluate `f(x) = x[0]`, so feedback varies with seeded input; arbitrary
distributions may still produce compressible values. The evaluator performs no
artificial arithmetic: it sleeps once per batch for `batch_size × eval_us`,
with seeded Gaussian jitter of 10% per batch. Zero delay has no sleep or jitter.
Generation, materialization, transport and accumulation still perform real work.

| Delay/sample | Evaluators | RNG samples/batch | Materialized/training samples/batch |
| --- | --- | ---: | ---: |
| 0 | Every doubling from 1 through 512 | 524,288 | 131,072 |
| 5 µs | 1, 4, 16, 64, 256, 512; also 128 for RNG and 8 for the other modes | 65,536 | 65,536 |
| 200 µs | 1, 4, 16, 64, 256, 512 | 4,096 | 4,096 |
| 5 ms | 1, 4, 16, 64, 256, 512 | 256 | 256 |

These are selected settings replayed for scaling, not a new optimization at every
point. Zero delay is an empirical reference, not a guaranteed upper bound for
other batch sizes. The ideal delay-limited rate is `evaluators / delay`.

The sampler owns its generation size. The preset fixes it per delay context,
bounded by the maximum batch size, sample allowance and 30 seconds of nominal
single-evaluator work. Runtime splitting uses a soft refill threshold of one
pending batch per evaluator; the threshold does not truncate draws.
Training uses a 10¹²-sample window, measuring feedback transport without optimizer
barriers. Finite training windows and real optimizers need separate tests.

The preset uses one sampler core, fifteen database/server cores, a 10 ms tick,
250 ms telemetry, six sampler connections, four concurrent inserts and five
batches per insert bundle. `sampler_io_threads` (1) and `sampler_db_pool_size` (6)
are explicit in the preset; older suites replay with one sampler I/O thread and
two connections. The new dedicated pool leaves control-plane I/O separate, so
replaying a suite does not recreate the historical process thread layout. The database uses 4GB of
shared buffers; older suites without `database_shared_buffers` retain 256MB.
Evaluator processes share the remaining physical cores when necessary. It needs
at least 17 physical cores. The allowance is 2,147,483,648 **samples**, not bytes:
the 512-worker RNG measurement used roughly 70 GiB of worker RSS, excluding
PostgreSQL and page cache. Reduce the fleet and `sample_memory_budget` on smaller
hosts; actual batch caps are recorded.

The default time limit is **60 minutes**, including deployment and cleanup.
`plan` reports minimum measurement time; warmup, draining and deployment add to
it. `--budget` changes the limit, never the validity rules.

### Measurement contract

- Readiness requires the assigned fleet and recent telemetry. RNG initializes
  its small Havana grid with one evaluator before attaching the full fleet.
- Warmup accepts all previously generated work, including buffered draw remainders,
  then two new batches per evaluator in aggregate. Its five-minute timeout is
  also bounded by the remaining suite budget.
- A confirmation spans at least 12 seconds, six nominal batch durations and ten
  batch durations divided by worker count. Training also spans two nominal
  generation cycles divided by worker count, because feedback arrives per draw.
- Adequacy requires eight accepted batches and eight evaluator completions in
  aggregate. Training must deliver feedback during the measured interval.
- Accepted progress and feedback use the same sampler telemetry endpoints and
  monotonic clock. Separately checkpointed run totals remain available.
- Assignment/task/epoch changes, stale or missing coverage, regressing counters,
  failed readiness and cleanup remain explicit failures. Missing data is not zero.

Reports show accepted samples/s, evaluator batches/s, batch sizes, four busy
fractions and worker RSS. Busy fractions are occupied wall time, including
simulated waiting and database waits; they are not CPU utilization. Compute and
I/O overlap. Short windows and jitter can put rates slightly above the ideal.

Selection prefers fewer evaluators and then shorter batches within 5% of the best
observed rate. Repeated confirmations use their median. This preference is not a
5% confidence guarantee. Use repeated matched A/B trials for deployment defaults
or causal speedup claims, especially at high evaluator counts.

### Targeted checks and exploration

`--points` accepts explicit configurations. Repeated entries produce independent
fresh-run confirmations:

```json
[
  {"mode": "training", "eval_us": 0, "workers": 16, "batch": 131072},
  {"mode": "training", "eval_us": 0, "workers": 16, "batch": 131072}
]
```

```sh
just benchmark frontier --points points.json --budget 600 --binary target/dev-optim/gammaboard --output results/check
```

Use this to rerun failed or unmeasured configurations without repeating the suite.
Retain original failures and provenance when combining studies.

`--search` is optional adaptive evaluator-batch and worker-count exploration.
It tunes live, drains pre-change work, checks larger/smaller batches, prunes
clearly dominated delayed-worker counts and confirms selected settings in fresh
runs. Zero delay retains every worker count. Queue refill depth remains fixed.
`--points` and `--search` are mutually exclusive; custom delays require one of them.

## Targeted process API measurements

```sh
just benchmark process --python /path/to/venv/bin/python --output results/process
just benchmark plot results/process
```

The driver builds optimized Rust integration tests using the production adapters,
Python SDK and v3 framed protocol. Parent and child occupy two physical cores.
Correctness checks cover weighted feedback, continuous/discrete dimensions,
feedback on/off, variable sizes, rejected requests and worker reuse.

The overhead sweep uses six continuous coordinates and one output component:

- Nine batch sizes: 16, 64, 256, 1,024, 4,096, 16,384, 65,536, 262,144, 1,048,576.
- Evaluator calls with feedback on/off; sampler generation and feedback separately.
- Zero or 64 in-place NumPy sine passes inside the callback, giving 72 cases.
- Three discarded warmups; 128 repetitions at small sizes, falling to four at 1M.

Each observation subtracts callback wall time from its matching adapter wall
time. The residual includes validation, packing, IPC, native conversion/
accumulation, deallocation and scheduling. Startup, fixture input construction,
the database and worker fleet are excluded. This is process-API overhead, not
wire-only IPC latency or a universal constant across domains and components.

Plots show mean overhead per call and per sample against batch size in PNG,
SVG and PDF. Raw paired timings, medians, startup times, affinity and source
hashes remain in the output. Shared-host noise is not a performance threshold.

## Focused I/O experiments

The default stress matrix compares 1/2/8 concurrent inserts, 1/8/32/64 evaluators,
16/256-sample batches and three repetitions. It uses 1 ms polling and at most
eight physical cores, exposing coordination pressure rather than CPU scaling.

```sh
just benchmark io --binary target/dev-optim/gammaboard --output results/io
just benchmark io --binary target/dev-optim/gammaboard --output results/payloads --workers 8 --batch-sizes 65536 262144 --inserts 1 2 8 --min-tick-ms 10 --duration 12 --repetitions 3
just benchmark plot results/payloads
```

Use a small matrix around the configuration under investigation. `--iterations`
adds fixed CPU work; `--insert-bundle-size` changes batches per transaction.
`--input-storage pglz|lz4|external` requires `psql` and changes only the private
database's input column. This is a deployment experiment, not live task tuning.

The workload sends materialized six-dimensional inputs and compact results,
without training barriers. Reports include samples/s, batches/s and logical input
MiB/s derived from measured payload bytes. Logical volume excludes results,
retries, framing, WAL and physical disk traffic. Local PostgreSQL uses
`synchronous_commit = false`; this is not a durability benchmark. Missing/changing
payload sizes and invalid busy counters invalidate the relevant measurement.

Insert concurrency counts in-flight tasks, not simultaneous database connections:
the sampler's role pool defaults to and is capped at six. Raising the insert limit
alone does not raise that connection limit. Leave capacity for result fetching
and maintenance. See [concurrency](concurrency.md).

## Fixed CPU-work comparisons

Optional CPU presets compare the pipeline with direct serial/parallel execution.
Calibration chooses a fixed arithmetic iteration count; reuse it across worker
counts and revisions. A delay is not a CPU-work baseline.

```sh
just benchmark plan resources/templates/benchmarks/smoke.toml
just benchmark run resources/templates/benchmarks/smoke.toml --binary target/dev-optim/gammaboard --output results/smoke
just benchmark run resources/templates/benchmarks/tuning.toml --binary target/dev-optim/gammaboard --output results/tuning
just benchmark compare results/before results/after
```

| Preset | Trials | Measurement | Budget | Purpose |
| --- | ---: | ---: | ---: | --- |
| `smoke.toml` | 4 | 4 s | 5 min | Runner, cleanup and artifacts |
| `tuning.toml` | 24 | 6 s | 15 min | Repeated batch-size comparisons at normal polling |
| `scaling.toml` | 48 | 6 s | 25 min | Fixed-batch CPU efficiency with the polling floor disabled |

All descendants share at most eight physical cores and at most a quarter of the
available cores. Direct workers bind individually; the pipeline also spends this
budget on the sampler/database. Repetitions randomize configuration order and
rotate direct/pipeline measurement order. Use
`--calibration previous/calibration.json` to keep arithmetic work fixed.

Direct evaluation also accepts a normal evaluator card:

```sh
gammaboard --json benchmark calibrate --eval-us 100
gammaboard --json benchmark evaluator evaluator.toml --workers 4 --batch-size 256 --warmup 2s --duration 5s
```

It uses production sampling/materialization/evaluation/accumulation with uniform
inputs and a scalar accumulator; it does not simulate a central adaptive sampler.
`speedup` compares against direct serial execution; `retained_efficiency` compares
against direct parallel execution within the same CPU budget. Trial ranges are
not confidence intervals from independent telemetry samples.

## Inspecting a live run

```sh
gammaboard --json run wait RUN --until ready --evaluators 4 --timeout 60s
gammaboard --json run performance RUN --duration 30s --interval 1s
gammaboard --json run performance RUN --since 2026-09-29T00:00:00Z --until 2026-09-29T01:00:00Z
gammaboard --json run wait RUN --until idle
```

Performance JSON has schema 1. Publications are asynchronous; intervals retain
snapshots, per-worker endpoints, coverage and issues. Default maximum telemetry
age is 10 seconds. Never sum overlapping busy times or average rolling means into
interval totals. Allocated core-time is worker allocation, excluding the
database/server, not measured CPU consumption. Historical exports have a
`truncated` flag; narrow the range when set. See [concurrency](concurrency.md)
for the four busy-rate definitions.

## Artifacts and comparisons

Deployment suites retain the binary, migrations, harness, cards, planned cases,
CPU placement, host metadata, raw intervals and cleanup evidence. Results append
after each trial. Offline `summary` and `plot` expose invalid and unmeasured
cases; a zero exit code alone does not establish complete coverage. Frontier
crosses mark unavailable planned points, never valid low rates.

Frontier outputs include throughput, batch-size, ideal-rate fraction and selected
configuration plots, plus `selected-*.toml` live batch overrides. Copy the matching
run card as well: a batch override alone does not reproduce sampler settings.

Current frontier artifacts use **schema 5**, with sampler-owned generation and
fixed refill depth. Use saved harnesses for older schemas. `compare` checks
workload, jitter/seed, feedback contents, resource/telemetry settings and matching
queue configurations; CPU suites also require matching calibrated work. Record
build profile and host conditions even when these checks pass. Process and
frontier measurements answer different questions and should not be pooled.

## Sampling correctness

Physics acceptance is separate from throughput:

```sh
GAMMABOARD_TEST_STATE_OUTPUT=/tmp/gammaboard-reference-state cargo test --locked --lib evaluation::evaluator::gammaloop::acceptance -- --nocapture
GAMMABOARD_TEST_REFERENCE_STATE=/tmp/gammaboard-reference-state cargo test --locked --test full_stack_cli full_stack_gammaloop_reference_training_and_inference -- --ignored --nocapture
```

The fixture output must be new. These checks generate/reload a version-10 scalar
cut-bubble state and test ordinary/cut-focused maps, summed/discrete channels,
normalization, moments, weighted histograms, feedback modes and recovery.
The pipeline uses a private database. Symbolica licensing is required; debug
timings are not a production baseline.

[The reference run](../resources/templates/runs/gammaloop-reference.toml) uses a
generated state without physical observables/selectors. Keep the map width
consistent with its momentum scale; check estimates, uncertainty and invalid
counts. Rebuild states and repeat acceptance after GammaLoop changes. Stability
retries and summed channels may perform several target evaluations per accepted
outer sample. Preparation timings remain subsets of evaluator compute.

The sparse suite does not establish GPU efficiency, multi-host scaling, optimizer
convergence or time to a physics uncertainty target. Use real-adapter,
finite-training-window and recovery tests for those claims.
