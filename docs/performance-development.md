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
- GammaLoop observable batches reset once at entry. The returned observable
  bundle owns its data, so a second full integrand clone after evaluation is
  unnecessary. Entry reset also isolates batches after mixed accumulator modes
  or failed evaluations.
- Worker and CLI processes default to one background I/O thread, with the
  existing `TOKIO_WORKER_THREADS` override. The API server keeps automatic
  sizing. MADNIS sets its one-thread OpenMP default in the Python entrypoint
  instead of repeating a 64-thread default in runtime packaging.

The real-physics reset regression uses a read-only generated state with
histograms. Set `GAMMABOARD_TEST_PHYSICS_STATE` and
`GAMMABOARD_TEST_PHYSICS_INTEGRAND`, then run:

```bash
cargo test --lib observable_batches_are_isolated_after_mixed_modes_and_failure -- --ignored
```

It compares complete results apart from timing fields across variable-sized
batches, scalar/vector/empty modes, and an external failure after a valid sample.

## Optimization candidates

These are code-review findings, not measured speedup claims. Keep each comparison
small and independent; do not change several scheduling mechanisms at once.

| Priority | Location | Finding | Next experiment |
| --- | --- | --- | --- |
| 1 | `src/evaluation/evaluator/gammaloop.rs`, `eval_batch` | The remaining entry reset clones the full pristine integrand. | Inspect whether GammaLoop can reset only observable state without changing evaluation caches or failure recovery. |
| 2 | `src/runners/queue.rs`, `tune_batch_size` | Smoothing, a deadband, and a cooldown all regulate batch sizing. Despite its `batch_size_cooldown_ticks` name, the cooldown advances when completed batches are observed, so its wall duration depends on batch completion rate. | Compare cooldown zero with the default on repeated training windows; retain it only if it reduces oscillation or improves throughput without delaying adaptation. Fixed-batch CPU presets intentionally bypass this code and cannot answer this question. |
| 3 | `src/runners/queue.rs`, insert/fetch pumps | The insert pump now has one enqueue/refill path. Comparing 1, 2, and 8 inserts did not establish a consistently better lower limit; the default remains 8. | Investigate claim/query contention at large evaluator counts, holding batch size and polling cadence fixed. Use the four activity rates and operation timings alongside accepted progress. |
| 4 | `src/evaluation/evaluator/gammaloop.rs`, `ingest_vector_batch` | Scalar/complex projection creates a small `Vec` per sample. | Profile allocation cost on a real integrand before replacing these with stack arrays or borrowed slices. Physics evaluation may dominate. |
| 5 | `src/sampling/generation.rs` and `src/runners/sampler_aggregator.rs` | Bulk generation trades fewer sampler calls for buffering, slicing, and retained training values. It is opt-in. | Compare bulk off/on at unchanged evaluator batch size; include a real process sampler before claiming a training benefit. Bound generation size to keep memory reasonable. |

## Insert scheduling experiments (2026-09-24)

The focused I/O preset completed 99 valid trials, three repetitions per case,
with one, two, or eight concurrent inserts. All processes shared eight physical
cores. The stress matrix used 1/8/32/64 evaluators, batches of 16/256 samples,
and 1 ms polling. Controls used normal 10 ms polling and deterministic CPU work.
See [benchmarking.md](benchmarking.md) for the reproducible command.

For the fast integrand with eight evaluators and batches of 256, normal-pacing
median accepted rates were 91k/140k/147k samples/s for 1/2/8 inserts. The CPU-work
control reached about 45k samples/s for all three limits, with evaluator compute
activity around 96%. Two inserts are a useful tuning option; one can restrict
cheap-batch throughput. Keep the default at eight until a lower bound has a
clearer benefit across workloads.

Batch size and polling mattered more than insert count in the stress cases.
The 64-evaluator normal-pacing repetitions varied widely (27k–135k samples/s),
with high sampler I/O activity in slower cases. These shared-host, fixed-core
tests expose a contention problem to investigate; they do not establish its
cause or measure CPU strong scaling. Check claim/query latency, database table
churn, and polling before adding threads or enlarging connection pools.

A follow-up with full six-dimensional inputs, batches of 65,536/262,144, and
eight evaluators completed 18 valid trials and accepted 6.21 GiB of serialized
input across the measurement windows. Configuration medians were 0.66–0.90
million samples/s (36–50 MiB/s of logical input). One insert was competitive;
larger batches did not improve throughput. Sampler I/O activity was 95–98%.
Live PostgreSQL observations caught evaluator completion updates and inserts
blocked by a large input COPY. Batch metadata updated a shared per-run queue
counter before that COPY, holding its lock until commit. The subsequent changes
below shorten that lock duration while preserving atomic batch/payload visibility.
This observation does not isolate the cause of the earlier small-batch slowdown.
The I/O preset now records batches/s and logical input volume alongside samples/s;
see the large-payload command in [benchmarking.md](benchmarking.md).

The new counters measure occupied wall time, including in-flight DB waits.
They exclude completed handles awaiting collection, and overlapping operations
count once. They are not CPU utilization or a requirement that every role stay
at 100%. See [concurrency.md](concurrency.md) for the exact scope.

## Queue throughput improvements

Three changes were retained after separate comparisons:

- Queue-counter triggers defer updates until commit, freeing the shared counter
  while payloads are written. Counters and batch changes still become visible
  atomically; rollback and cascading deletion are covered by database tests.
- Input serialization and COPY-buffer preparation precede connection acquisition
  and the transaction. Binary serialization borrows the input arrays instead of
  cloning them; golden fixtures preserve existing stored encodings.
- New input writes use LZ4 where PostgreSQL supports it. Unsupported builds keep
  their existing compression with a migration warning. WAL compression and
  durability settings are unchanged.

With eight evaluators, two inserts, normal polling, and three repetitions,
median accepted rates changed as follows (million samples/s):

| Stage | Batch 256 | Batch 65,536 | Batch 262,144 |
| --- | ---: | ---: | ---: |
| Fresh baseline | 0.141 | 0.868 | 0.753 |
| Deferred counters | 0.142 | 1.327 | 1.179 |
| Payload preparation | 0.153 | 1.351 | 1.048 |
| LZ4 | 0.151 | 2.293 | 1.834 |
| Uncompressed comparison | 0.152 | 2.103 | 1.605 |

These shared-host trials establish a useful development result, not a universal
speedup. Preparation reduced observed serialization cost but did not establish
an independent throughput gain. Small-batch throughput ranges overlapped.
One-batch insert bundles did not improve large-batch medians and substantially
reduced small-batch throughput. No byte-based bundling policy was added; bundle
size 5 and insert concurrency 8 remain the defaults. Further tuning was stopped
at the user's request. See [benchmarking.md](benchmarking.md) for reproduction
options and migration provenance.

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
