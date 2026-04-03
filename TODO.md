# TODO

## Platform
- [ ] implement `madnis` sampler-aggregator as a parametrization
- [ ] instrument and optimize `insert_batches` end to end, especially the `batch_inputs` write path
- [ ] investigate training-mode queue payload bloat vs Havana inference seed payloads
  - `LatentBatchPayload::Batch` stores a full `Batch` of concrete points, while Havana inference only stores a `u64` seed
  - `insert_batches(...)` serializes every latent batch synchronously inside the hot path before the `batch_inputs` insert
  - `claim_batch(...)` decodes the full latent batch again on every evaluator fetch
- [ ] investigate completed-batch fetch/decode hot path in training mode
  - `fetch_completed_batches(...)` always joins `batches` with `batch_results` and loads both `values` and `batch_observable`
  - `PgStore::fetch_completed_batches(...)` immediately decodes `values` bytes and deserializes the observable JSON for every completed batch
  - inference mode strongly suggests this path is materially cheaper when payloads are smaller / training values are absent
- [ ] investigate `runs.current_observable` write cost as the remaining inference-mode sampler bottleneck
  - `flush_aggregation(false)` still serializes `observable_state.to_json()` on every frontend sync
  - `save_aggregation(...)` still performs a full `UPDATE runs SET current_observable = $1` even when the queue path is otherwise idle
- [ ] investigate whether training-mode completed-batch handling is doing unnecessary work per batch
  - `process_completed()` merges each batch observable one by one in the sampler hot path
  - completed batches are deleted immediately after processing, so hot-path queue turnover still pays synchronous delete cost
- [ ] measure serialized payload sizes directly for produced latent batches and completed batch results
  - compare naive / Havana training / Havana inference byte sizes for `latent_batch`, `values`, and `batch_observable`
  - current timings strongly suggest payload size is a first-order cause, but the code does not surface byte metrics yet
- [ ] PYo3 wrapper for generic python based integrand
- [ ] PYo3 wrapper for generic python based sampler
- [ ] add pdf to sampler, use it to plot integrand vs pdf in dashboard
- [ ] let the user save tasks and run tomls.

## Dashboard
- [ ] adjustable ranges for all plots
- [ ] extend image plots: complex Plotly image trace with phase-hue / magnitude-intensity legend
- [ ] export svn/json/whatever of all plots buttons
- [ ] Reorder plots, e.g. progress more at top, tasks also at top, then right below that the live averages.
- [ ] import json of histograms and compare them to current.
