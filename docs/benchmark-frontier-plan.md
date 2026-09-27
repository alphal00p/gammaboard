# Proposed capability benchmark suite

Discussion summary, 27 September 2026. **Implementation is deferred at the
user's request while other questions are investigated.** This is a proposed
design, not an implemented suite or a set of measured results. Revisit it before
starting work; publication packaging and release freezing remain later tasks.

## Purpose and parameters

Produce a faithful, concise account of GammaBoard's maximum throughput and where
larger batches or more evaluators stop providing useful returns.

| Parameter | Proposed candidates |
| --- | --- |
| Evaluation cost | 0.5, 5, 50 microseconds; 0.5, 5, 50 milliseconds per sample |
| Evaluator count | 1, 2, 4, 8, 16, 32, 64, 128, subject to available resources |
| Target batch evaluation duration | 2, 20, 200, 2,000 milliseconds; refine near transitions |
| Main workload | Calibrated fixed CPU work, full six-dimensional inputs, scalar inference results |

Keep calibrated work unchanged across worker counts and compare against matched
direct execution. Record achieved batch sizes and durations, not just targets.
Add a short zero-artificial-work, fixed-batch sweep to measure the data-movement
and scheduling ceiling independently of batch adaptation.

## Adaptive search instead of a full Cartesian sweep

1. Survey the six costs using sparse worker counts (initially 1, 8, 32, 128)
   and widely spaced batch targets.
2. Skip settings that collapse to the same effective batch size through minimum,
   maximum, or training-window limits.
3. Add intermediate worker counts and batch targets where scaling flattens,
   batching helps substantially, the best target changes, or measurements are
   inconclusive. Refine evaluation costs around regime changes if needed.
4. Before pruning a worker-count plateau, test whether larger batches restore
   scaling. Keep occasional probes beyond the apparent plateau to avoid missing
   another improvement. Pruning must account for trial variation.
5. Independently confirm candidate knees and their neighbors with three
   repetitions. Report configurations within 5% of the best measured throughput,
   identifying the fewest workers and shortest batches without implying those
   two choices necessarily form the same configuration. Treat differences below
   observed variation as equivalent or unresolved. If the best result is at a
   search boundary and still improving, report "frontier not reached."

Proposed budgets: 60 minutes including startup and cleanup, or 20 minutes for
exploration. These are scheduling limits, not guaranteed completion estimates.
Reserve time for confirmation and stop exploration first. Discard controller
stabilization; aim for at least eight completed batches per evaluator rather
than imposing one short duration on every case. Save incomplete/invalid cases
and pruning reasons; mark unexplored regions explicitly in reports and plots.

## Training and minibatches

Use a smaller training study around inference knees at cheap, intermediate,
and expensive evaluation costs. Expand it if training shifts the knee markedly.
Vary minibatch size and update duration, including a zero-added-delay control.
Hold minibatch size fixed while varying evaluator count: scaling it with workers
would conceal the limited parallelism between updates.

Measure accepted samples/s and completed training cycles/s over at least five
complete cycles. Include representative generation/ingestion costs and a CPU
MADNIS check. Synthetic update delays model barriers and service time; they do
not establish CPU/GPU training efficiency or convergence quality.

## Constraints and gaps to address before running

- Calibration and suite validation currently start at 1 microsecond and need
  support for fractional-microsecond targets, verified against actual timings.
- Normal batches have a 16-sample minimum: at 50 ms/sample the minimum duration
  is about 800 ms. Training also caps batch sizes to distribute a finite window
  across evaluators. Do not treat unattainable targets as distinct experiments.
- Existing runner CPU limits, wall-time budgets, database connection admission,
  and slow-batch telemetry freshness need to support the proposed cases.
  Synchronous evaluation can delay publication beyond the configured cadence.
- Large CPU scaling runs require enough available physical cores, with a fixed,
  separate sampler/database allowance and explicit total resource accounting.
  The host exposed 256 physical cores during inspection, but it is shared;
  availability is not a reservation. Running 128 evaluators inside eight cores
  measures contention, not CPU strong scaling.
- Keep polling, storage/durability, payload shape, telemetry, and other tuning
  fixed and disclosed. Add a few GammaLoop/MADNIS checks to ground synthetic
  claims. Multi-host/GPU scaling, large result payloads, startup amortization,
  and time to a target uncertainty remain separate coverage gaps.

## Report and plots

Keep the narrative short, with configuration, raw data, and validity details
available alongside it. Produce:

- Throughput/scaling curves by evaluation cost and batch target, including the
  cheap-evaluation throughput ceiling.
- Efficiency relative to matched direct execution.
- A map of useful worker counts and batch durations across evaluation costs.
- Training throughput versus minibatch size/update cost, plus one four-rate
  activity timeline showing minibatch pauses.

Distinguish measured, confirmed, skipped, and unresolved points. Show variation
between independent repetitions; do not manufacture precise frontiers or
confidence intervals from correlated telemetry snapshots.
