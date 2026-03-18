# TODO

## Platform Features
- [ ] add basic auth to dashboard which enables steering (spawning virtual workers, creating runs etc.)
- [ ] implement madnis sampler_aggregator as parametrization.

## Sweep Findings (2026-03-04)

### Performance
- [ ] avoid per-event single-row DB writes in telemetry sink; batch inserts (`INSERT ... VALUES (...)` in chunks or `COPY`) to reduce write amplification.
- [ ] optimize `fetch_completed_batches` contiguous-prefix query (`ROW_NUMBER` over oldest rows each tick); track a per-run cursor/last-consumed batch id to avoid repeated rescans.
- [ ] implement persistnent observable which is leaner thant the observable for observable history, the full observable is only needed for the history, the persistent observable can be a more compact representation that is merged into the full observable when consumed by the history.
- [ ] implement pausing a run by serializing the sampler_aggregator. When pausing a run, we would need to make sure to handle the batch queue

### Duplication Of Code
- [ ] consolidate frontend object/number helpers (`toObject`, `toObjectOrNull`, local numeric parsing helpers) into shared utility modules.
- [ ] deduplicate repeated CLI command-name mapping and tracing-span setup patterns across `run.rs` and `node.rs`.
- [ ] deduplicate API handler boilerplate in `src/server/mod.rs`: repeated `limit.clamp(1, 10_000)`, store fetch, `Option` to 404 conversion, and `serde_json::to_value(...)` response wrapping.
- [ ] extract shared CLI bootstrap in `src/cli/*` for `init_pg_store(...)`, `init_cli_tracing(...)`, and span instrumentation used by `run.rs`, `node.rs`, `run_node.rs`, and `server.rs`.
- [ ] collapse repeated run lifecycle control flow in `src/cli/run.rs`: `start`, `pause`, `stop`, and `remove` all branch on `selection.all` and then iterate with near-identical logging/update patterns.
- [ ] introduce a shared dashboard API request helper in `dashboard/src/services/api.js` for endpoint path building, query-string assembly, error handling, and optional normalization.
- [ ] replace repeated polling state machines in `dashboard/src/hooks/useRuns.js`, `useWorkersData.js`, `useRunPerformanceSummary.js`, and `useWorkerPerformanceHistory.js` with a reusable `usePolledResource`-style hook.
- [ ] unify implementation-panel registry dispatch in `dashboard/src/components/evaluator/EvaluatorCustomPanel.jsx` and `dashboard/src/components/sampler/SamplerCustomPanel.jsx`, or fold both into the shared panel infrastructure when the remaining custom implementation views are retired.
- [ ] factor shared control-plane worker SQL in `src/stores/queries/control_plane.rs`: repeated `workers` update clauses for clearing desired assignments and repeated fixed control-plane row defaults.
- [ ] add shared row-decoding helpers in `src/stores/queries/read.rs` for repeated bigint-to-string id conversion, JSON metric parsing, and default-on-decode-failure behavior.

## Sweep Findings (2026-03-18)

### Complexity
- [ ] split `src/server/task_panels.rs` by task kind or adapter role; it still mixes sample-task aggregate decoding, deterministic full-observable rendering, descriptor definitions, and current/history projection in one file.
- [ ] introduce a shared deterministic full-observable task adapter for `image` and `plot_line`; both tasks currently duplicate progress/current-from-persisted/current-from-runtime wiring and only differ in geometry-specific rendering.
- [ ] extract a small aggregate-observable panel builder for sample tasks; scalar/complex estimate, summary, and history projection still live as ad hoc helper sets inside `src/server/task_panels.rs`.
- [ ] replace repeated frontend polling hooks with a shared `usePolledResource` abstraction that covers scheduling, stale-response protection, and `isConnected`/error handling consistently.
- [ ] remove remaining silent JSON serialization fallbacks in `src/stores/queries/work_queue.rs`; several `serde_json::to_value(...).unwrap_or_default()` calls still hide persistence errors instead of surfacing them.
- [ ] investigate the failing `just test-e2e` control-plane reassignment timeout (`timed out waiting for sampler replaces second evaluator`) and either fix the transition logic or make the failure mode explicit.
