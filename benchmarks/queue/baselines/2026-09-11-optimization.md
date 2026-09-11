# Queue optimization experiments — 2026-09-11

Keep **10 ms polling**, **one pending batch per evaluator**, and the **2-second
batch target**. Replace the refill low/high ratios and local-buffer multiplier
with one pending-work target covering database, local and in-flight batches.

## Repeated comparisons

| Experiment | Result | Decision |
| --- | --- | --- |
| 50 → 10 ms polling, 4 evaluators, fast inference | +17.1% mean paired rate; three pairs, +10.3% to +21.8% | Keep 10 ms |
| Same polling change, bursty training | +23.8%; three pairs, +23.1% to +24.3% | Keep 10 ms |
| Remove three buffer controls, 4/64 evaluators | Inference about −3.5%/−3.3%; training +3.7%/−0.9%, two pairs each | Accept simpler policy; small tradeoff, not a speedup claim |
| Pending target 1 → 2, corrected build, 64 evaluators | Inference pairs −3.8%/+15.4%; training −1.3%/+0.01% | Keep 1; no consistent gain to justify more buffering |
| Combined changes, **2-second batch target**, 4 evaluators, fast inference | +4.5%; three pairs, +3.5% to +5.5% | Confirms benefit beyond the short-suite batch target |
| Combined changes, slow 64-evaluator burst workload, 20-second windows | A 148.7–161.4/s; B 161.2–161.4/s | No sustained regression; the short-window drop below did not repeat |

Fast workloads have a nominal compute ceiling of 2,000,000 samples/s; the slow
case uses 1,000/s. Training always includes repeated **0.5-second update stalls**.
The 2-second-target check retains 100 ms progress snapshots for measurement;
it does not reproduce every production persistence interval.

## Full matrix: coarse screen

All 32 A/B measurements completed in **204.9 seconds**, including deployment and
cleanup. These use the standard 100 ms batch target and 4-second measurement
windows. Single-pair changes, particularly at batch/update boundaries, are noisy:
the apparent 34% slow-training regression disappeared in the longer check above.

| Case | A samples/s | B samples/s | Change |
| --- | ---: | ---: | ---: |
| inference-e1-r1000 | 991 | 969 | -2.2% |
| training_burst-e1-r1000 | 292 | 308 | +5.4% |
| inference-e1-r2000000 | 521,783 | 616,479 | +18.1% |
| training_burst-e1-r2000000 | 110,771 | 142,326 | +28.5% |
| inference-e4-r1000 | 976 | 970 | -0.6% |
| training_burst-e4-r1000 | 258 | 336 | +30.3% |
| inference-e4-r2000000 | 760,670 | 825,894 | +8.6% |
| training_burst-e4-r2000000 | 114,665 | 149,623 | +30.5% |
| inference-e16-r1000 | 970 | 970 | -0.0% |
| training_burst-e16-r1000 | 209 | 312 | +49.5% |
| inference-e16-r2000000 | 515,663 | 829,687 | +60.9% |
| training_burst-e16-r2000000 | 83,662 | 148,313 | +77.3% |
| inference-e64-r1000 | 583 | 583 | -0.0% |
| training_burst-e64-r1000 | 185 | 122 | -34.0% |
| inference-e64-r2000000 | 662,664 | 650,453 | -1.8% |
| training_burst-e64-r2000000 | 99,468 | 116,724 | +17.3% |

## Correctness and simplification

- CPU accounting no longer writes the shared task row for activity or desired
  assignment changes. A 64-worker pause exposed a deadlock with heartbeats;
  accounting remains on lease, capability and actual-assignment updates.
- Concurrent inserts could commit out of order and let the completion cursor
  skip a late bundle. The larger-buffer experiment lost five batches of training
  feedback. Fetches now stop before outstanding or subsequently started inserts;
  a deterministic regression covers both races. Corrected repeated runs finish.
- Three queue controls, their dashboard fields, redundant diagnostics and template
  copies are removed. Batch-size stabilization, training-window boundaries,
  retry limits and queue/I/O bounds remain.
- Benchmark sockets use a short private temporary directory, avoiding PostgreSQL's
  socket-path limit with long output paths; cleanup was verified.
- PostgreSQL store tests require an explicit database URL and fail on connection
  errors. An omitted URL during this work created 14 fixtures in another local
  deployment before schema errors stopped the tests. Those exact unused fixtures
  were removed; the isolated database run passed. There is no silent default or
  successful test skip now.

## Method and artifacts

Both executables use `dev-optim`, without GammaLoop, on the same local host with
OMP/Rayon limits of 8. GL26 assignments were paused during measurements. A is the
frozen consolidation baseline (engine code `0ea7f08`); B contains the queue
implementation committed as `fe7a988`. Both sides after the first polling experiment receive the CPU-trigger
fix, so that correctness fix is not credited as a measured throughput gain.
Paired repetitions reverse A/B order and share seeds. Compilation and unrelated
tests were kept outside measurement windows.

Each completed invocation took **107–213 seconds**, below the five-minute budget.
Failed attempts (pause deadlock, skipped feedback, long socket path) are excluded
from throughput conclusions and retained alongside the successful runs. This is
a local, synthetic service-time benchmark; it does not model HPC network costs.
Shorter polling increases idle query frequency and remains configurable through
`min_tick_time_ms`.

[Machine-readable results](2026-09-11-optimization.json) include exact overrides,
binary hashes, paired ratios and completion times. Raw run cards, traces and logs
are retained in the sibling `setup-logs/` directories with the experiment names
in that file. Use `scripts/benchmark_queue.py ab` with the recorded workload and
sampling settings to repeat the code comparison.

Validation: locked upstream `cargo check`, formatting, 235 library + 8 CLI tests
without GammaLoop; 239 library + 8 CLI tests with the local GL26-compatible
dependencies; 17 PostgreSQL tests; 17 dashboard tests; 8 benchmark tests; and
**42 end-to-end tests passed** (optional Apptainer case excluded). Native GL26
checks used the existing local dependency overrides, then restored the tracked
manifests. The first full-stack attempt missed a mid-training pause because the
run finished before publishing intermediate progress. That test now uses explicit
synthetic timing and frequent progress snapshots; the misplaced implicit TOML
polling override helper was removed. The complete rerun passed in 54 seconds.

Live-run follow-up: the dashboard and three ordinary workers were restarted.
Restoring GL26 run 1 from its older checkpoint failed with no unprocessed batches
and no further sampler production. The failed run and stage snapshots are retained
for recovery investigation; its training was not reset. This live recovery failure
is separate from the completed synthetic benchmarks and passing end-to-end suite.
