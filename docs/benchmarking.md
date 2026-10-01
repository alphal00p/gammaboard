# Benchmarks

One Python CLI runs the measurements and immediately writes separate PNG/SVG
plots, CSV summaries and raw JSON, with small local HTML reports for navigation. Training feedback off/on is
included automatically. There are no optimizer-update or end-to-end correctness
tests in this suite.

## Run

On Linux, use an optimized GammaBoard binary, PostgreSQL (`initdb`, `pg_ctl`,
`psql`), `taskset`, and Python 3.11+ with NumPy and matplotlib. The repository
checkout supplies the harness, migrations and Python SDK fixture. Neither nginx,
a built dashboard, a running GammaBoard instance nor a Rust test executable is
required. The benchmark creates and removes its own local database.

```bash
cargo build --locked --profile dev-optim --bin gammaboard
python3 -m venv .venv
.venv/bin/pip install numpy matplotlib
.venv/bin/python -m benchmarks all --quick --output results/quick
```

`just benchmark` is an optional shorthand for `python3 -m benchmarks`.
Use `--binary PATH` when the executable is elsewhere; `CARGO_TARGET_DIR` is
honored. `--python PATH` selects the protocol child interpreter.

```bash
just benchmark plan
just benchmark frontier --output results/frontier
just benchmark sampler-io --output results/sampler
just benchmark evaluator-io --output results/evaluator
just benchmark protocol --output results/protocol
```

Open `results/quick/report.html` after an `all` run. Reports are produced even
when measurements fail or a budget expires, with unavailable coverage explicit.
A full frontier is the longest family; `plan` shows measured time before setup,
warmup and cleanup. Quick mode uses frontier counts 1, 4, 16 and 64 by default (override the maximum
with `--max-evaluators`) and uses one 2-second I/O
trial per setting, compared with two 4-second trials normally. Large batches
extend the measured interval to target at least sixteen completions (bounded at
30 seconds or the requested duration for sampler I/O). It does not weaken
frontier readiness or minimum-sample requirements. Repeats reverse case order
in the same private database; cache state and background database work can affect
rates. These trials are not isolated cold-database comparisons.

Useful overrides:

```bash
just benchmark all --quick --max-evaluators 16 --cpu-limit 12 --output results/laptop
just benchmark sampler-io --io-threads 1 2 4 8 --output results/io-threads
just benchmark sampler-io --io-threads 4 --database-cores 16 --profile --output results/profile
just benchmark protocol --batch-sizes 16 1024 32768 131072 --output results/protocol-short
```

`--memory-mib` bounds estimated I/O payload residency (default 2048 MiB); the
PostgreSQL cache and executable/runtime memory are additional. Largest batches
need room for at least eight batches including transient copies. Reduce batch
sizes on smaller machines. `--cpu-limit` bounds physical core allocation; sampler,
evaluators and PostgreSQL use disjoint sets. Worker processes may share evaluator
cores in delayed frontier cases. Large registered fleets also need memory for
workers and database connections: reduce `--max-evaluators` on smaller machines.
These tests do not reserve an otherwise idle host.

## Coverage

| Family | Measured path | Main output |
| --- | --- | --- |
| Frontier | Actual sampler and evaluator runners, queues, bookkeeping and accumulation | Accepted samples/s against evaluator count |
| Sampler I/O | Prepared materialized batches → production insert → lightweight consumers → production result fetch/decode and cleanup | Samples/s against batch size and I/O thread count |
| Evaluator I/O | Prefilled queue → production claim/decode → prepared result → production submit | Samples/s against batch size, with one I/O thread |
| Protocol | Production Rust process adapters and Python SDK; no database | Adapter overhead per batch and per sample |

All materialized cases use six continuous dimensions and scalar results.
Input coordinates vary within each batch; returned training values are `x[0]`,
avoiding constant-value feedback compression. Prepared I/O inputs/results are
reused between batches. These are database-path capacities, including PostgreSQL
and serialization, not pure network bandwidth. They exclude runner tick policy,
checkpoint writing and sampler/evaluator arithmetic; the frontier includes those
runtime effects.

Training means transporting and ingesting per-sample feedback. Model optimization,
GPU performance, finite-minibatch pauses, large observable states and multi-host
network behavior are outside this suite's default scope.

## Frontier

The default sparse measurement retains evaluator doublings from 1 through 512 on
the zero-delay curve, and sparse anchors on the 5 µs, 200 µs and 5 ms curves.
Delays are simulated by one sleep per batch with seeded 10% Gaussian jitter;
there is no artificial CPU arithmetic. Nominal batch time is capped at 30 seconds. The zero-delay materialized/training
curve uses 32,768 samples per evaluator batch. The default transport cap is
32,768, separately from the 4,194,304-sample generation cap. Delayed curves use
32,768 / 4,096 / 256 samples at 5 µs / 200 µs / 5 ms; timing and memory bounds can
reduce these values. Generation stays large enough to amortize draws.

Materialized inputs with feedback off/on are the default pair. Add `--include-rng`
for compact RNG inference. The preset is
[`frontier.toml`](../benchmarks/frontier.toml).
`--workers 1 16` selects explicit counts; `--points FILE.json` selects exact
`mode`, `eval_us`, `workers`, `batch` points from the effective suite. These are
scaling curves at selected batch/queue settings, not a new optimization search at
every point. The old adaptive search and CPU-work matrices have been removed.

Warmup passes all previously generated work, including a buffered generation.
Accepted sample deltas and elapsed time use the same sampler telemetry endpoints.
Coverage and identities must remain stable, telemetry must be fresh, and at least
eight accepted batches and evaluator completions must be observed. Longer delayed
batches receive longer intervals. Every mode observes at least two nominal generation
cycles because concurrent insert bundles and ordered collection can produce bursts
even without feedback. Accepted and evaluated progress must agree within 10% or
two batches. If they disagree, use the first and last observed complete generation
boundaries when they span at least half the observation and eight batches. The
untrimmed observation is always saved. Unresolved intervals receive one longer
retry and remain flagged if the backlog boundary still distorts the measurement.
Failed and unmeasured configurations never become zero-throughput observations.

The report shows the peak observed configurations, preferring fewer evaluators
and smaller batches when within 5% of the peak. Effective settings, workload cards,
resource assignments and exact raw intervals are saved. Memory-dependent batch
caps are explicit; do not compare runs with different settings as code-only changes.

## Amortization

`python -m benchmarks amortization --output results/amortization` runs a separate
matched CPU-work study; it is not added to the default `all` sweep. Defaults are
1 and 16 evaluators, five transport batch sizes (128, 512, 2,048, 8,192, 32,768),
feedback off/on and three independently restarted repeats. `--workers`,
`--batch-sizes`, `--duration` and `--repetitions` select smaller experiments.
The default deployment requires 25 physical cores: five sampler, four database
and sixteen evaluator cores. CPU affinity is fixed across the paired methods;
the host is not reserved exclusively.

One short calibration selects a fixed arithmetic iteration count targeting about
5 µs/sample, then holds that count fixed for every case. There are no sleeps.
Both methods use the same native six-dimensional uniform sampler, evaluator
returning `x[0]`, scalar accumulation and optional ordered generation feedback.
Generation size is fixed at 131,072, independent of transport batching. Feedback
is transport and ingestion; this is not an adaptive-model or GLNIS benchmark.

The direct reference uses a bounded in-memory worker pipeline with two outstanding
batches per evaluator. It includes generation, partitioning, materialization,
evaluation, accumulation and generation-level feedback, but no database, storage
serialization or production runners. It reuses the numerical engines; it is not
a comparison against a separately optimized physics implementation or a measure
of every instruction in the GammaBoard library. The production side uses the
normal queue and runner measurement contract, four sampler I/O threads and fixed
queue settings. Warmup and final draining are excluded on both sides.

Extra steady-state runtime is `100 × (direct_rate / gammaboard_rate − 1)` for
equivalent accepted sample counts. The horizontal axis is measured direct
evaluator time per batch, including local accumulation. Rates use their own
monotonic completion windows. Small-batch runner windows are extended to cover
feedback cycles even when the 10 ms runner tick dominates useful CPU work.
Negative measured overhead is retained, never clipped. Lines show median paired
results and faint dots show independent repetitions; CSV retains the full range.

The command writes `amortization.png`, `.svg`, `.pdf`, CSV/JSON summaries and a
small local HTML report. Run cards, exact windows, direct measurements, calibration,
CPU allocations, source/binary provenance and cleanup outcomes are retained.

## I/O capacities

Default batch sizes are 256, 4096, 16,384, 65,536 and 131,072 samples. Sampler I/O uses
four concurrent insert tasks, up to five batches per bundle, and a six-connection
pool. Sixteen lightweight consumers drain real inputs and return real results;
`--consumers` can check partner headroom. Sampler I/O sweeps 1, 2, 4 and 8 threads
by default, keeping two cores available for the database and consumers; smaller CPU budgets trim only the default sweep.
Explicit `--io-threads` values must fit the CPU budget. All cases in a sweep use
the same disjoint role CPU sets, with enough sampler cores for the largest count.
The frontier retains the thread count in its preset unless explicitly overridden.
Bounded outstanding work prevents backlog growth from masquerading as sustainable
throughput. Throughput counts collected results.

Evaluator I/O measures one worker with one-batch lookahead and a two-connection
pool. Each pass is fully prefilled before timing and leaves two reserve batches.
Passes are repeated until the requested measured duration is covered. Prefill,
cleanup and warmup are excluded. This is a warmed database/cache measurement;
queue starvation invalidates it. Every timed pass is retained and combined by
its actual duration.

Plots show throughput only, with separate feedback-off/on panels. Lines show
medians and faint dots show repeated trials. Busy remains in the saved
measurements and CSV summaries. The timer is shared with production:
executing I/O counts, including database waits; overlapping operations count once.
Waiting for work/channel capacity and completed operations awaiting collection do
not count. Busy at or below 90% prints a warning and marks the trial as unsaturated;
it does not discard an otherwise valid rate. High busy alone does not establish
hardware utilization: check saved consumer activity, empty claims and
publication/completion windows as well.

Reports include batches/s and encoded input/feedback MiB/s. Byte rates do not
include network framing, WAL or physical disk traffic. Raw windows and effective
buffer/bundle sizes accompany each measurement and appear in the summary.
The sampler defaults to 64 outstanding batches and five batches per insert. All
sizes in the default sweep fit this same queue and bundle within 2048 MiB.
Overrides beyond the default range may reduce queue/bundle sizes to respect the
memory budget; the report records those effective limits.

Optional resource probes use `--database-cores`, `--database-cache-mib`,
`--insert-concurrency`, `--queue-batches` and `--consumers`. The insert pool has two
connections beyond the insert concurrency. These are benchmark controls, not new
production configuration fields. `--profile` samples PostgreSQL active-backend
wait states every 100 ms during measured windows. Raw data also includes completed
operation times for serialization, metadata insertion, payload COPY, commit,
result fetch and cleanup. These times overlap across operations: their sums are
not busy percentages. Database-state counts are sampled observations, not CPU
utilization measurements. `--database-directory PATH` chooses the filesystem for
the temporary private database; its default is `/tmp`. A RAM-backed directory is
a diagnostic of storage headroom, not a durable deployment recommendation.

## Process protocol

The normal optimized executable runs the real adapters against the bundled Python
SDK fixture. No Cargo invocation or test binary is needed at measurement time.
Parent and child run on separate physical cores. Logarithmic batch sizes range
from 16 to 131,072, including the 32,768 default; three warmup calls are discarded
and 32–128 calls are measured. Larger cases retain at least 32 calls so a few
scheduling delays do not dominate the result.

Each adapter call is paired with its callback's elapsed time. Their difference
includes validation, packing, pipes, native conversion/accumulation, destruction
and scheduling. It is **adapter overhead**, not pure IPC latency. Startup and raw
paired timings are recorded separately. Sampler training cycles combine generation
and feedback at the same batch size.

Sampler calls retain their native `Generation`, including the training window;
result cleanup is included for both roles. `Generation::into_batch()` only unwraps
a flat `LatentBatchSpec`; it is not the per-sample expansion performed by
`LatentBatchPayload::into_batch()`. Removing that wrapper does not eliminate a
per-sample conversion. Earlier documentation incorrectly conflated the two.

The generic evaluator returns per-sample values over IPC in both modes because
Rust performs accumulation. Feedback-on additionally retains weighted values;
similar protocol curves in the two modes are expected. Functional process tests
remain in `tests/process_api.rs` and are not part of the benchmark command.

The [2026-10-01 protocol comparison](benchmarks/2026-10-01/protocol-overhead/README.md)
records the corrected baseline, individual adapter changes, before/after plots
and a targeted end-to-end confirmation.

## Results

Every family creates `report.html`, plots, `summary.csv`, `summary.json`, a manifest
and raw measurements. `all` creates a small index linking the separate family
reports; images are linked files, not embedded copies. A shared `inputs/` directory
preserves the executable, benchmark package and migrations once per suite. The
protocol family also saves its Python SDK and fixture. Interrupted runs retain logs and failure status; successful runs stop
workers and remove their private database. To regenerate reports after changing
the presentation:

```bash
just benchmark report --output results/frontier
```

The superseded `run`, `io`, `process`, `plot` and `summary` CLI commands and CPU
matrix presets are removed. Historical studies retain their original saved harness.
Use `gammaboard --json run performance RUN --duration 30s` for a live deployment.

## Batch-size finding

Larger transported batches eventually reduce throughput. In the October 1
six-dimensional materialized sweep, 65,536-sample batches outperformed
1,048,576-sample batches at every tested thread count, with and without feedback.
The old 2 GiB memory budget also reduced queue slots from 128 to 12 and insert
bundles from five batches to one at the largest size. That confounded batch size
with buffering, so it was not evidence of a universal database size threshold.

The diagnostic I/O sweep focuses on 256–131,072 samples and holds sampler
buffering constant. The run and frontier defaults now cap transported batches at
32,768; bulk generation is independent. Existing October 1 frontier results used
65,536-sample zero-delay batches and have not been remeasured at the new default.
These defaults can be overridden: explicit larger I/O/protocol sizes remain supported up to
1,048,576, and a custom frontier preset can raise its cap. Bulk generation retains
its independent 4,194,304-sample cap. Recheck the knee for different dimensions,
feedback payloads, machines and database settings. Resource comparisons and the
full rerun are recorded in [performance development](performance-development.md).

## Code organization

The Python package [`benchmarks/`](../benchmarks/) owns the experiment policy:

| File | Responsibility |
| --- | --- |
| `__main__.py`, `cli.py` | Single entry point, defaults and family orchestration |
| `common.py` | Private database lifecycle, CPU allocation, input snapshots, shared counters |
| `frontier.py`, `frontier.toml` | Sparse workload plan and accepted-progress measurement |
| `frontier_plots.py` | Frontier figures and selected deployment settings |
| `io.py` | Both database-path sweeps, validity checks and throughput reports |
| `protocol.py` | Paired process-adapter measurements and overhead reports |
| `reporting.py` | Local HTML navigation, CSV and figure output |
| `tests/` | Measurement, aggregation and cleanup contracts |

[`src/benchmark/`](../src/benchmark/) contains only the Rust paths needed to
exercise production stores and process adapters. It returns raw measurements;
Python owns planning, warnings, summaries and plots. The shared Python process
fixture remains in `process_api/python/tests/runtime_fixture.py`, beside SDK
tests. Functional adapter tests remain in `tests/process_api.rs`; they do not
carry benchmark matrices. No compatibility wrappers or old search presets remain.

Run harness tests with `python -m unittest discover -s benchmarks/tests`.
