# Sampler generation and feedback

```text
sampler.generate(min(queue.max_generation_size, remaining task budget))
  → bounded draw → adaptive evaluator batches → parallel evaluation
  → ordered results → one feedback(values) call for the original draw
```

`generate(max_samples)` receives a positive per-draw limit and returns
`Generation::Batch`, `Waiting`, or `Finished`. A draw contains 1..max_samples
samples. The runtime bounds each call by the queue's `max_generation_size`
(default 262,144) and any remaining task budget. Samplers may return fewer
samples, for example at a training boundary.
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

Set `max_generation_size` under `[sampler_aggregator_runner_params.queue]`,
or override it in task `queue_tuning` or the dashboard. A live change takes
effect on the next draw; buffered draws and their feedback boundaries remain
intact. This is a per-draw cap, not a total queue or memory cap. All built-in
samplers use the supplied limit, clipped to their training window or remaining
raster points. Sampler authors need no independent generation-size setting
and do not implement evaluator batch tuning.

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
Process workers use `gammaboard-jsonrpc-v4` and must update to the bundled SDK.
Old v2/v3 workers fail the protocol handshake rather than failing mid-run.
Update custom sampler callbacks from `generate(remaining_sample_budget)` to
`generate(max_samples)`; the limit is now always a positive integer. Evaluator
callbacks and binary payload layouts are unchanged. Move native sampler
`generation_batch_size` settings to queue `max_generation_size`; retired values
in saved sampler snapshots are ignored so the current queue limit takes effect.

Removed settings (`bulk_sample_generation`, `queue_buffer`, `max_queue_size`,
`batch_size_deadband_ratio`, `batch_size_cooldown_ticks`) are ignored when loading
old configuration and omitted on export. Task tuning contains the duration,
maximum evaluator batch size, maximum generation size, and fixed-size override. Insert/fetch and per-tick
limits remain deployment settings, not task-level controls. Older checkpoint
metadata is restored where its sampler implementation remains compatible;
external sampler snapshots must satisfy the new pending-feedback contract.
