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
- [ ] reduce run-stream DB load: only publish SSE when run progress or aggregated snapshot changed since last emission.
- [ ] add visibility gating to `useRuns` polling (runs list still polls at fixed cadence even when tab is hidden).
- [ ] reduce expensive recomputation in `ObservableSection` (`history.slice().reverse().map(...)` every update); maintain pre-derived series incrementally.
- [ ] optimize `WorkerLogsPanel` merge path (currently map+sort over full set on each update); use monotonic append fast-path when ids are ordered.
- [ ] `WorkersPanel` polling should support request cancellation and skip/abort stale in-flight fetches during rapid run switches.

### Code Simplicity
- [ ] split `RunHistoryContext.jsx` into smaller hooks (`useRunSse`, `useRunPolling`, `useRunVisibility`) with one reducer-driven state model.
- [ ] split `WorkersPanel.jsx` into focused components/hooks (table, diagnostics, polling) to reduce file size and cognitive load.
- [ ] centralize repeated polling-loop boilerplate (abort controller, timeout scheduling, error handling) in a shared frontend hook.
- [ ] replace ad-hoc JSON parsing/merging in `run add` (`merge_json`, `parse_run_add_payload`) with typed config structs and serde validation.
- [ ] reduce telemetry parsing complexity by using a compact typed context extraction path and keeping fallback JSON capture minimal.
- [ ] factor shared CLI selector handling (`all` vs explicit IDs) into reusable helpers for run/node command handlers.

### Duplication Of Code
- [ ] consolidate frontend object/number helpers (`toObject`, `toObjectOrNull`, local numeric parsing helpers) into shared utility modules.
- [ ] consolidate custom-panel dispatch pattern (`EvaluatorCustomPanel`, `SamplerCustomPanel`, `ObservableCustomPanel`) behind one generic registry helper.
- [ ] deduplicate repeated integration-param extraction logic used in `RunInfo`, `EvaluatorPanel`, `SamplerAggregatorPanel`, and `ObservablePanel`.
- [ ] deduplicate similar polling loops in `useRuns` and `RunHistoryContext` with one shared scheduling primitive.
- [ ] deduplicate repeated CLI command-name mapping and tracing-span setup patterns across `run.rs` and `node.rs`.

### Legacy / Unnecessary Code
- [ ] remove `dashboard/src/utils/sampleParser.js` (currently empty).
- [ ] remove or wire unused CRA leftovers (`dashboard/src/logo.svg`, `dashboard/src/reportWebVitals.js`) if not part of current runtime/test flow.
- [ ] audit `runtime_logs` columns `node_id` and `request_id`: currently schema keeps them, but telemetry insert path does not populate them.
- [ ] remove backward-compat fallback parameter keys in frontend panels if backward compatibility is intentionally dropped.
- [ ] resolve remaining `//todo` placeholders in `gammaloop` evaluator (`validate_point_spec`, `supports_observable`) or explicitly mark as intentionally deferred.
- [ ] evaluate whether `pub mod batch { pub use crate::core::batch::*; }` alias in `lib.rs` is still needed or only legacy indirection.

### Inconsistencies In Naming And Logic
- [ ] standardize backend port env var naming (`GAMMABOOARD_BACKEND_PORT` contains a typo relative to project name) and propagate consistently.
- [ ] align docs with implementation for `spherical` parametrization (README still describes hemispherical mapping; code uses direct spherical map with radial blow-up).
- [ ] align metric naming semantics (`sum_weight` currently stores weighted value sum, not a pure weight sum).
- [ ] align run progress labels in UI (`processed_batches_total` vs `batches_completed` vs queue counts) to avoid semantic ambiguity.
- [ ] unify naming style between `run-node` CLI command and `run_node` internal module/span names where exposed in logs/docs.
- [ ] remove stale tracing compatibility behavior (`target="worker_log"` fallback path) once span-based context is fully enforced everywhere.
