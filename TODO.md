# TODO

## Platform Features
- [ ] add basic auth to dashboard which enables steering (spawning virtual workers, creating runs etc.)
- [x] implement gammaloop evaluator
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

### Legacy / Unnecessary Code
- [x] remove or wire unused CRA leftovers (`dashboard/src/logo.svg`, `dashboard/src/reportWebVitals.js`) if not part of current runtime/test flow.
- [x] audit `runtime_logs` columns `node_id` and `request_id`: currently schema keeps them, but telemetry insert path does not populate them.
- [x] remove backward-compat fallback parameter keys in frontend panels if backward compatibility is intentionally dropped.
- [x] evaluate whether `pub mod batch { pub use crate::core::batch::*; }` alias in `lib.rs` is still needed or only legacy indirection.

### Inconsistencies In Naming And Logic
- [x] standardize backend port env var naming (`GAMMABOOARD_BACKEND_PORT` contains a typo relative to project name) and propagate consistently.
- [x] align docs with implementation for `spherical` parametrization (README still describes hemispherical mapping; code uses direct spherical map with radial blow-up).
- [x] align metric naming semantics (`sum_weight` currently stores weighted value sum, not a pure weight sum).
- [x] align run progress labels in UI (`processed_batches_total` vs `batches_completed` vs queue counts) to avoid semantic ambiguity.
- [x] unify naming style between `run-node` CLI command and `run_node` internal module/span names where exposed in logs/docs.
- [x] remove stale tracing compatibility behavior (`target="worker_log"` fallback path) once span-based context is fully enforced everywhere.
