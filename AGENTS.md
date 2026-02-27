# AGENTS

## Purpose
This file is for coding agents and contributors making structural or behavioral changes.
Use `README.md` for human/operator onboarding, and use this file for repo-internal rules.

## System Snapshot
- `control_plane` manages runs and desired role assignments.
- `run_node` is role-agnostic and reconciles DB desired assignments into one active local role task.
- `server` exposes read APIs and SSE for the dashboard.
- PostgreSQL is the source of truth for queue, control-plane state, leases, and snapshots.
  - Worker performance history is persisted in role-split append-only tables:
    `evaluator_performance_history` and `sampler_aggregator_performance_history`.

## Module Ownership
- `src/core/*`
  - Shared domain and store-facing contracts.
  - Batch/point domain types (`Batch`, `BatchResult`, `PointSpec`), `RunStatus`,
    worker/assignment models, store traits, `StoreError`.
- `src/engines/*`
  - Runtime engine contracts and implementations.
  - Role-aligned submodules own contracts/factories: `engines::evaluator`,
    `engines::sampler_aggregator`, `engines::observable`,
    `engines::parametrization`.
  - Shared engine wiring/types live in `engines::shared`
    (`BuildFromJson`, `IntegrationParams`, `RunSpec`).
  - Engine implementation enums, run spec/integration params parsing, engine errors.
  - Implementation enums derive string/display behavior via `strum`
    (`AsRefStr`, `Display`) and should be rendered with `.as_ref()` or `{impl}` formatting.
- `src/runners/*`
  - Orchestration loops (`NodeRunner`, evaluator runner, sampler-aggregator runner).
  - `node_runner` also owns typed runner parameter structs used in run spec decoding
    (`EvaluatorRunnerParams`, `SamplerAggregatorRunnerParams`).
- `src/stores/*`
  - Postgres implementation and read-side DTOs/traits.
- `src/telemetry.rs`
  - Global tracing subscriber setup + DB sink for worker log events.
- `src/bin/*`
  - Operational binaries only (`control_plane`, `run_node`, `server`).

## Operational Conventions
- Run configuration is passed as TOML to `control_plane run-add`.
- `control_plane run-add` requires an explicit TOML file path argument (no embedded default payload).
- Run identity is configured via top-level `name` in TOML and persisted in `runs.name`.
- Engine/runner settings are persisted in `runs.integration_params`; point shape is persisted in `runs.point_spec`.
- Observable implementation is persisted in `runs.observable_implementation`.
- `control_plane run-pause` / `run-stop` should set run status (`paused`/`cancelled`)
  and clear desired assignments for targeted runs so `run_node` supervisor reconciliation
  stops active role tasks without requiring workers to poll run status.
- `control_plane` run lifecycle commands (`run-start`, `run-pause`, `run-stop`, `run-remove`)
  should use a shared selector shape: `-a|--all` or one-or-more positional `RUN_ID`s.
- `control_plane node-stop` should use the same selector shape (`-a|--all` or positional `NODE_ID`s)
  and request node shutdown via `workers.shutdown_requested_at`.
- Parametrization implementation and params are persisted in `runs.integration_params`
  as `parametrization_implementation` and `parametrization_params`.
- Keep `configs/live-test*.toml` as explicit reference configs: include all runner/engine fields,
  even when values equal defaults.
- Name live-test scenario configs semantically (describe intent/compatibility), not only by index.
- Keep a Symbolica live-test reference scenario for a 2D polynomial integral
  (`configs/live-test-symbolica-unit-square-polynomial-scalar.toml`) to exercise
  evaluator codegen/compile/load in end-to-end runs.
- Batch payloads in `batches.points` must stay compact and shape-stable:
  row-major flat `continuous`/`discrete` arrays, per-sample `weights`, and
  explicit 2D shape metadata.
- Evaluators operate batch-wise (`Batch -> BatchResult`), where `BatchResult` contains
  weighted training `values: Vec<f64>` and one aggregated batch-level observable JSON.
- Evaluator implementations receive an `ObservableFactory` in `eval_batch` and build
  per-batch observable instances through the factory.
- Evaluator and sampler-aggregator engines can emit run-scoped initialization metadata
  via trait hooks (`Evaluator::get_init_metadata`, `SamplerAggregator::get_init_metadata`);
  runners persist these once per run in `runs.evaluator_init_metadata` and
  `runs.sampler_aggregator_init_metadata` (write only when column is `NULL`).
- Observable ingestion in evaluators should use capability methods on `Observable`
  (`as_scalar_ingest`, `as_complex_ingest`) with default `None`, rather than
  matching concrete observable engine enum variants.
- Evaluator runner applies optional parametrization (`Batch -> Batch`) before calling
  `Evaluator::eval_batch`; parametrization is selected per run via
  `parametrization_implementation` + `parametrization_params`.
- `spherical` parametrization maps unit-hypercube continuous samples to unit-ball
  coordinates and updates batch weights by the spherical-coordinate Jacobian.
- Observable construction should be centralized via `engines::observable::ObservableFactory`
  (shared by evaluator and sampler-aggregator runners), not by passing raw implementation
  enum + params through call boundaries.
- `symbolica` evaluator codegen artifacts should be created in per-engine temporary
  directories and owned by the evaluator instance so they are cleaned up when the
  evaluator is dropped (best-effort cleanup on normal process shutdown).
- Sampler-aggregator engines produce one batch per call (`produce_batch`); the runner owns
  per-tick multi-batch production loops and queue-capacity limiting.
- Runner-controlled sample count per produced batch comes from
  `sampler_aggregator_runner_params.nr_samples`; runners pass this value directly to
  `SamplerAggregator::produce_batch`.
- Havana sampler params must not include `batch_size`; Havana also uses
  `sampler_aggregator_runner_params.nr_samples` as the per-batch sample count.
- Sampler-aggregator engines may optionally throttle per-tick batch production via
  `SamplerAggregator::get_max_batches` (default `None` means no engine-specific cap).
  Havana uses this to enforce deterministic update-cycle limits while training is active.
- Havana training-rate config is scheduled via absolute `batches_produced`:
  `initial_training_rate` -> `final_training_rate` (exponential interpolation), typically
  bounded by required `stop_training_after_n_batches` (training stop only;
  production continues).
- Sampler-aggregator engines may return optional local in-memory batch context
  (`BatchContext`) from `produce_batch`; the runner stores it keyed by `batch_id`
  and passes it back to `ingest_training_weights`.
- Batch context is process-local only (not persisted to DB); implementations must
  tolerate missing context after restart.
- Runs specify evaluator/sampler/observable/parametrization implementations independently.
- Evaluator/sampler/parametrization implementation names remain in `integration_params`; observable implementation is in `runs.observable_implementation`.
- Runtime engine construction should use role-specific factories
  (`EvaluatorFactory`, `SamplerAggregatorFactory`, `ParametrizationFactory`,
  `ObservableFactory`) returning boxed trait objects; do not add runtime
  `*Engine` dispatch enums.
- Runner params in `IntegrationParams`/`RunSpec` are strongly typed
  (`EvaluatorRunnerParams`, `SamplerAggregatorRunnerParams`) instead of raw JSON blobs.
- Concrete engine implementations should parse JSON params through `engines::BuildFromJson` (typed params + validation) instead of ad-hoc per-engine parsing helpers.
- `BuildFromJson` implementations define only `type Params` and `from_parsed_params`; shared JSON decoding/error wrapping lives in `BuildFromJson::from_json`.
- Keep compatibility rules in typed implementation enums and validate at
  startup (for evaluator/observable: `Evaluator::supports_observable`).
- `scalar` observable state is `ScalarObservable` (serde-derived) and tracks
  `count`, `sum_weight`, `sum_abs`, and `sum_sq` over evaluator values.
- `complex` observable state is `ComplexObservable` (serde-derived). Treat current
  merge behavior as implementation-defined/incomplete unless explicitly finalized.
- `complex` observable must expose both ingestion capabilities:
  `ComplexIngest` directly and `ScalarIngest` via `real -> complex(real, 0)` casting.
- Observable payload handling should use serde-derived structs (`Serialize`/`Deserialize`) plus
  `Observable::{load_state_from_json, merge, snapshot}`; avoid manual `json!` object
  construction and field-by-field `Value::get` merging in observable implementations.
- Observable aggregation merge is observable-to-observable (`merge(&dyn Observable)`): load
  completed batch JSON into a freshly built observable instance, then merge that observable
  into the run-level aggregate observable.
- Completed batches are consumed by sampler-aggregator and deleted from `batches`; there is no persisted sampler engine state checkpoint.
- Evaluator and sampler-aggregator performance stats are accumulated in-memory and
  flushed as periodic history snapshots (`performance_snapshot_interval_ms`) into:
  `evaluator_performance_history` and `sampler_aggregator_performance_history`.
- Engine diagnostics for those snapshots should be emitted via trait defaults/hooks:
  `Evaluator::get_diagnostics()` and `SamplerAggregator::get_diagnostics()`
  (default `json!("{}")`).
- Latest worker stats shown in `/api/workers` are read from role-specific latest views:
  `evaluator_performance_latest` and `sampler_aggregator_performance_latest`.
- Both history tables include a `diagnostics` JSONB payload for implementation-specific
  diagnostics (for example optimizer/loss metadata in future engines).
- `run_node` role is controlled by DB desired assignments for its `node_id`; CLI does not select role.
- A `run_node` process executes at most one active role task at a time.
- Operational stop flows should prefer control-plane desired-state changes
  (`run-stop`/`run-pause`) over process-kill shutdowns so workers reconcile down cleanly.
- `run_node` execution model is supervisor + worker task: outer poll loop reconciles desired assignment and starts/stops one spawned role task for the current run.
- `run_node` should consume node shutdown requests (`workers.shutdown_requested_at`) as one-shot
  signals: clear request, stop current role task, then exit process.
- Worker runtime state is encapsulated in `src/runners/node_runner/` (`NodeRunner`/`ActiveWorker`) and owns its store handle.
- Keep role switching safe: stop old role task, then start new one.
- Worker registration metadata uses run-spec implementation strings and binary
  version (`CARGO_PKG_VERSION`).
- `run_node` initializes tracing with DB persistence enabled; only events with
  `target="worker_log"` are written to `worker_logs`.
- Log events intended for dashboard visibility should include at least:
  `run_id`, `worker_id`, `role`, `event_type`.
- Read API includes `GET /api/runs/:id/logs` (optional `limit`, `worker_id`, `level`).
- Read API includes `GET /api/workers` with optional `run_id` filter; payload
  includes worker registration fields plus optional run-scoped performance stats.
- Read API includes history endpoints:
  `GET /api/runs/:id/performance/evaluator` and
  `GET /api/runs/:id/performance/sampler-aggregator`
  (optional `limit`, `worker_id`).
- Run stats SSE (`GET /api/runs/:id/stream`) uses one shared per-run backend polling
  task with fanout/broadcast to subscribers; avoid per-client DB polling loops.
- Schema migration policy (current): no backward-compat requirements. Prefer
  direct table definitions for current schema and avoid follow-up `ALTER TABLE`
  compatibility migrations unless explicitly requested.

## Required Checks Before Finishing
- `cargo fmt`
- `cargo check -q`
- `cargo test -q`

## Documentation Sync Rule
If a change affects structure or operations, update docs in the same change:
- Always update `AGENTS.md` for internal architecture/conventions.
- Always update `README.md` for user-facing workflows/commands.

Changes that require doc updates include:
- module moves/renames
- binary/CLI changes
- config schema changes (TOML fields)
- migration/schema changes that affect runtime behavior
- run orchestration or control-plane behavior changes
