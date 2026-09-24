# Performance work during development

Use the small benchmark preset that answers the current question. Keep its raw
inputs and measurements, but do not freeze development or prepare release
artifacts just to investigate a change. Publication follows functional stability
and complete, repeatable measurements.

## Simplifications in this pass

- One CLI-based benchmark runner owns deployment, readiness, measurement,
  teardown, and reports. The separate sleep-based queue runner and SQL-based
  campaign script duplicated that lifecycle and maintained separate SQL-based
  measurement semantics outside the CLI contract.
- Shared run/task examples replace copied UBELIX physics cards. Cluster-specific
  worker launch cards remain in ops. The Slurm helper no longer has a separate
  recursive database deletion command.
- Sampling tasks initialize their accumulator directly. The main GammaLoop
  example and appended training/inference workflow now exclude training from
  published results and initialize a fresh inference accumulator.
- The frontend's build cache reinstalls npm dependencies when the package files
  change. An existing Vite executable is not sufficient evidence that installed
  dependencies match the lockfile.

## Optimization candidates

These are code-review findings, not measured speedup claims. Keep each comparison
small and independent; do not change several scheduling mechanisms at once.

| Priority | Location | Finding | Next experiment |
| --- | --- | --- | --- |
| 1 | `src/evaluation/evaluator/gammaloop.rs`, `reset_observables` / `eval_batch` | GammaLoop observable batches clone the full pristine integrand both before and after evaluation. The post-batch clone appears redundant because the next observable batch resets at entry. | Compare one reset with two on a compatible physics state. Verify repeated batches, histogram totals, mixed accumulator modes, and recovery after a failed evaluation before removing it. Longer term, inspect whether GammaLoop can reset only observable state. |
| 2 | `src/runners/queue.rs`, `tune_batch_size` | Smoothing, a deadband, and a cooldown all regulate batch sizing. Despite its `batch_size_cooldown_ticks` name, the cooldown advances when completed batches are observed, so its wall duration depends on batch completion rate. | Compare cooldown zero with the default on repeated training windows; retain it only if it reduces oscillation or improves throughput without delaying adaptation. Fixed-batch CPU presets intentionally bypass this code and cannot answer this question. |
| 3 | `src/runners/queue.rs`, insert/fetch pumps | Several limits bound pending work and concurrent inserts. More concurrency can increase database work when batches are cheap. | First vary batch size with `tuning.toml`; then compare one versus the default concurrent insert tasks using a focused run card. Measure accepted progress and database latency, not just evaluator rate. |
| 4 | `src/evaluation/evaluator/gammaloop.rs`, `ingest_vector_batch` | Scalar/complex projection creates a small `Vec` per sample. | Profile allocation cost on a real integrand before replacing these with stack arrays or borrowed slices. Physics evaluation may dominate. |
| 5 | `src/sampling/generation.rs` and `src/runners/sampler_aggregator.rs` | Bulk generation trades fewer sampler calls for buffering, slicing, and retained training values. It is opt-in. | Compare bulk off/on at unchanged evaluator batch size; include a real process sampler before claiming a training benefit. Bound generation size to keep memory reasonable. |

## Mechanisms to preserve

Claim tokens, retained results during database retries, task-scoped consumption,
and checkpointed in-flight generation protect correctness. They are not removable
performance tuning. Similarly, the fixed single-slot evaluator prefetch/submit
pipeline provides bounded overlap; deeper buffering should require evidence.

The direct baseline is deliberately measured again for every case. Caching a
single baseline across a shared-host run would save time but hide load changes.
Reduce the experiment matrix before introducing that shortcut.

## Measurement order

1. Run the smoke preset after runner or deployment changes.
2. Use the tuning preset for batch-size and polling questions (about ten minutes).
3. Run scaling when the relevant mechanism is stable (about twenty minutes).
4. Add real-physics and training comparisons for claims outside warm CPU inference.

Use `just benchmark summary OUTPUT` to expose missing and invalid repetitions.
Preserve failed runs as diagnostics; do not treat them as zero throughput or
quietly discard them from a performance claim. See [benchmarking.md](benchmarking.md).
