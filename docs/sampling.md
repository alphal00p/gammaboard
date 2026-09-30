# Sampler generation and feedback

```text
sampler.generate(remaining task budget)
  → sampler-sized draw → adaptive evaluator batches → parallel evaluation
  → ordered results → one feedback(values) call for the original draw
```

`generate(remaining_sample_budget)` returns `Generation::Batch`, `Waiting`, or
`Finished`. `None` is an unlimited task budget. A draw must contain at least one
sample and must not exceed the remaining budget. The budget is a task limit,
not a suggested draw size or queue capacity. The sampler chooses its own size.
`Waiting` means it needs outstanding feedback before it can draw again;
`Finished` means no more draws, while outstanding work still completes.

A batch's `training_remaining: Some(n)` requests feedback and reports how many
samples remain to be drawn in its current training window, including this draw.
It must be at least the draw size. `None` requests no feedback. A sampler without
a larger training barrier can report the draw size. This metadata helps the
runtime leave enough evaluator chunks for parallelism; it does not trigger a
model update.

Generation draws, evaluator batches, and training windows are independent.
For example, a 20,000-sample training window can use four 5,000-sample draws,
each split into smaller evaluator batches. The runtime returns four feedback
calls. The sampler updates its model only after the window is complete.

## Queue and batch sizing

A new draw starts below one pending batch per active evaluator. Pending includes
local buffered work and in-flight publication. This threshold is deliberately
soft: an existing draw continues dispatching and can exceed it. The runtime
retains at most one undispatched draw, while queued draws may still await results.
A large draw therefore trades fewer sampler calls for memory and queued work.

The runtime adapts evaluator chunks to `target_batch_eval_ms`, bounded by
`max_batch_size` and by the draw remainder. Neither setting limits a generation
draw. An advanced `fixed_batch_size` override bypasses timing adaptation. An
initial small probe discovers the accumulator layout before the remaining draw
is dispatched. Internal batch tuning uses a 15% deadband and three completed-batch
observations between changes. Finite training windows aim for four chunks per
active evaluator, subject to the minimum chunk size.

Naive Monte Carlo and native Havana default to 1,048,576 samples per draw,
clipped to the task budget and training window. Their sampler config accepts
`generation_batch_size` for workloads that need a different memory/call tradeoff.
MadNIS uses its existing `max_batch_size` as the generation size. Sampler authors
do not need to implement evaluator batch tuning.

## Feedback, memory, and recovery

`feedback(values)` receives exactly one scalar per sample of the oldest
feedback-bearing draw, in its original sample order. It contains the configured
training projection with transport/sampler weights already applied once.
It is not the complete evaluator output or a copy of the coordinates.
Havana divides out the sampler weight before its native training API applies it.
Inference draws receive no feedback.

Evaluators can finish out of order. The runtime accepts results in production
order and collects fragments until a complete draw is available. The scalar
array costs about 8 MB per million samples, in addition to the sampler's own
training state and transport buffers. Havana retains native Samples (including
nested weights) until feedback and reuses a completed sample buffer. MadNIS
retains the tensors it needs for training.

A checkpoint records the sampler snapshot, undispatched draw and split cursor,
partially collected feedback, and queue/progress cursors together. Sampler
snapshots must retain all private state needed to apply outstanding feedback.
Completed training draws must not be replayed after restoration. External model
artifacts retain the existing snapshot limitations; this change does not add
artifact versioning.

Native Havana inference keeps RNG checkpoints every 1,024 samples instead of
materializing coordinates in the sampler. Arbitrary evaluator splits preserve
the sample stream, with at most 1,023 skipped samples replayed by the evaluator.
Old single-seed queue payloads remain readable.

## Migration

The former planning/count/ordinary/bulk-generation methods are replaced by
`generate` and `feedback`; bulk generation is now the only runtime model.
Process workers use `gammaboard-jsonrpc-v3` and must update to the bundled SDK.
Old v2 workers fail the protocol handshake rather than failing mid-run.

Removed settings (`bulk_sample_generation`, `queue_buffer`, `max_queue_size`,
`batch_size_deadband_ratio`, `batch_size_cooldown_ticks`) are ignored when loading
old configuration and omitted on export. Task tuning contains only the duration,
maximum evaluator batch size, and fixed-size override. Insert/fetch and per-tick
limits remain deployment settings, not task-level controls. Older checkpoint
metadata is restored where its sampler implementation remains compatible;
external sampler snapshots must satisfy the new pending-feedback contract.
