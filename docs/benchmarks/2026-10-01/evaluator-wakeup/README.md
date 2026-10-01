# Evaluator wakeups — 2026-10-01

Keep one scheduling rule: process available work immediately; an empty evaluator
waits for the sampler to signal a queue refill. A staggered 50–100 ms safety
retry handles missed hints, requeues and reconnects. There is no productive-work
sleep, growing empty-poll backoff, or new tuning setting.

The sampler broadcasts once after the current input refill has committed, using
its existing insert-task bookkeeping. The evaluator listens and claims through
one of its existing two role connections; the other remains available for results
and telemetry. Subscribe before the first claim, discard old hints before each
fresh queue check, and always let the existing claim token/lease SQL decide
ownership. Waiting for a hint is idle, not I/O busy. No new thread, queue table,
or connection budget is needed. Evaluator tick/pool settings are retired; older
run cards remain readable. The sampler's 10 ms tick is unchanged.

## Why this fixes the regression

Removing the 10 ms evaluator floor made fast consumers reach empty queues sooner.
The old exponential polling delay could then outlast the next sampler refill:
new work did not wake the evaluator. Short-generation throughput repeatedly fell
in the [earlier experiment](../evaluator-pacing/README.md).

Simplifying/moving polling delays did not reliably remove that regression.
Notifying on every input bundle fixed short queues but caused a broadcast herd
at 128 evaluators: one pilot lost 16% throughput while transaction traffic almost
doubled. Notifying only on the database counter's empty-to-nonempty transition
still produced too many signals when consumers drained every bundle. Coalescing
notifications per refill removes that repeated fan-out without new scheduler
state. A fixed fallback period was also replaced by jitter to avoid synchronized
idle checks. [Exploration data](exploration.json) retain these discarded trials.
One intermediate comparison was stopped when unrelated host load rose sharply;
its rates are excluded from the final results below.

## Final end-to-end comparison

Before is the 10 ms evaluator scheduler with the peer/age claim restriction
already removed. After adds coalesced notifications and removes evaluator pacing.
Both use the same optimized Rust 1.94 profile. Correctness and lint checks use
Rust 1.99, matching the current CI toolchain. The final binary is immutable and
identified by SHA-256 in [the manifest](manifest.json).

| Workload | Feedback | Before | After | Median paired change |
| --- | --- | ---: | ---: | ---: |
| 128 fast evaluators; B=32,768 | off | 5.468 M/s | 5.958 M/s | +9.6% |
| 128 fast evaluators; B=32,768 | on | 5.249 M/s | 5.725 M/s | +9.1% |
| 1 fast evaluator; four batches/draw | off | 1.332 M/s | 1.477 M/s | +10.9% |
| 1 fast evaluator; four batches/draw | on | 1.358 M/s | 1.469 M/s | +8.2% |
| 1 CPU evaluator; B=512 | off | 46.04 k/s | 153.22 k/s | +232.8% |
| 1 CPU evaluator; B=512 | on | 46.16 k/s | 150.79 k/s | +226.7% |
| 256 slow evaluators; B=16 | off | 5.12 k/s | 5.11 k/s | -0.1% |
| 256 slow evaluators; B=16 | on | 5.10 k/s | 5.13 k/s | +0.6% |
| 128 evaluators; 5 s training pauses | on | 0.79 k/s | 0.79 k/s | +0.2% |

Rates are medians across the recorded repeats; the last column is the median of
paired changes, not the ratio of group medians. Fast-fleet and short-generation
cases have two pairs in reversed order; the other cases have one pair. These are
short engineering comparisons on a shared host, not publication confidence
intervals. Materialized fast-fleet repeats in particular vary; do not treat their
median gain as a precise ceiling improvement.

All inputs have six continuous coordinates; training returns nonconstant x[0]
values. Fast cases have zero artificial evaluator delay. Small CPU batches use
3,051 arithmetic iterations/sample. Slow evaluators sleep 50 ms/sample, with
seeded 10% batch-duration jitter: their expected limit is about 5,120 samples/s.
They test GammaBoard scheduling and database pressure, not CPU strong scaling.
The training-pause case updates for 5 s after each 4,096-sample window.

Each trial uses a fresh private PostgreSQL deployment, 1 GiB shared buffers,
eight database cores, five sampler cores/four I/O threads, four concurrent inserts,
and the same 51-core evaluator budget. Large fleets share those evaluator cores.
The final run has no overlapping study build/test job. Boundary core-load samples
and per-worker observations are preserved in the raw local artifacts.

## Database pressure and lifecycle

| Workload | Feedback | Transactions/s, before → after | DB CPU cores, before → after |
| --- | --- | ---: | ---: |
| 128 fast evaluators; B=32,768 | off | 5789 → 4956 | 3.88 → 3.90 |
| 128 fast evaluators; B=32,768 | on | 5697 → 4983 | 4.33 → 4.32 |
| 256 slow evaluators; B=16 | off | 3131 → 3248 | 1.32 → 1.28 |
| 256 slow evaluators; B=16 | on | 3080 → 3228 | 1.44 → 1.36 |
| 128 evaluators; 5 s training pauses | on | 4812 → 4554 | 0.77 → 0.64 |

Small CPU batches process over three times as much work, so absolute DB traffic
increases, while transactions per sample decrease. CPU/transaction counters
bracket the CLI observation and can cover a slightly different interval from
accepted-progress telemetry; they are diagnostic measurements, not additive
components of overhead. No deadlocks or temporary-file writes were observed.
All 26 measurements passed the progress/window checks and all 26 deployments
shut down cleanly.

Validation: 355 normal Rust tests, 37 PostgreSQL integration tests, the real
notification cancellation/race regression, both CLI smoke checks, 41 benchmark
unit tests, formatting, Rust 1.99 Clippy with all targets/features and warnings
as errors, and the build without default features. The notification regression
is included in CI. See [validation details](validation.json), [paired summaries](summary.json)
and [all measurements](results.json).

Raw local measurements, immutable binaries, source snapshots and the study driver:
`/common/dev/cedric/setup-logs/evaluator-idle-cadence-20261001/`.
