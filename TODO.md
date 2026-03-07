# TODO

## Platform Features
- [ ] add basic auth to dashboard which enables steering (spawning virtual workers, creating runs etc.)
- [ ] implement panel when no run specified, add tab for "worker view" showing all workers and their current runs/roles/stats
- [ ] implement madnis sampler_aggregator

## Sweep Findings (2026-03-04)

### Performance
- [ ] avoid per-event single-row DB writes in telemetry sink; batch inserts (`INSERT ... VALUES (...)` in chunks or `COPY`) to reduce write amplification.
- [ ] optimize `fetch_completed_batches` contiguous-prefix query (`ROW_NUMBER` over oldest rows each tick); track a per-run cursor/last-consumed batch id to avoid repeated rescans.
- [ ] reduce expensive recomputation in `ObservableSection` (`history.slice().reverse().map(...)` every update); maintain pre-derived series incrementally.

### Duplication Of Code
- [ ] consolidate frontend object/number helpers (`toObject`, `toObjectOrNull`, local numeric parsing helpers) into shared utility modules.
- [ ] deduplicate similar polling loops in `useRuns` and `RunHistoryContext` with one shared scheduling primitive.
- [ ] deduplicate repeated CLI command-name mapping and tracing-span setup patterns across `run.rs` and `node.rs`.
