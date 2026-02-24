# AGENTS

## Purpose
This file is for coding agents and contributors making structural or behavioral changes.
Use `README.md` for human/operator onboarding, and use this file for repo-internal rules.

## System Snapshot
- `control_plane` manages runs and desired role assignments.
- `run_node` is role-agnostic and reconciles DB desired assignments into one active local role task.
- `server` exposes read APIs and SSE for the dashboard.
- PostgreSQL is the source of truth for queue, control-plane state, leases, and snapshots.

## Module Ownership
- `src/core/*`
  - Shared domain and store-facing contracts.
  - Batch/point domain types (`Batch`, `BatchResult`, `PointSpec`), `RunStatus`,
    worker/assignment models, store traits, `StoreError`.
- `src/engines/*`
  - Runtime engine contracts and implementations.
  - Role-aligned submodules own contracts/factories: `engines::evaluator`,
    `engines::sampler_aggregator`, `engines::observable`.
  - Shared engine wiring/types live in `engines::shared`
    (`BuildFromJson`, `IntegrationParams`, `RunSpec`).
  - Engine implementation enums, run spec/integration params parsing, engine errors.
- `src/runners/*`
  - Orchestration loops (`NodeRunner`, evaluator runner, sampler-aggregator runner).
- `src/stores/*`
  - Postgres implementation and read-side DTOs/traits.
- `src/telemetry.rs`
  - Global tracing subscriber setup + DB sink for worker log events.
- `src/bin/*`
  - Operational binaries only (`control_plane`, `run_node`, `server`).

## Operational Conventions
- Run configuration is passed as TOML to `control_plane run-add`.
- Engine/runner settings are persisted in `runs.integration_params`; point shape is persisted in `runs.point_spec`.
- Observable implementation is persisted in `runs.observable_implementation`.
- Batch payloads in `batches.points` must stay compact and shape-stable:
  row-major flat `continuous`/`discrete` arrays, per-sample `weights`, and
  explicit 2D shape metadata.
- Evaluators operate batch-wise (`Batch -> BatchResult`), where `BatchResult` contains
  training `values: Vec<f64>` and one aggregated batch-level observable JSON.
- Evaluator implementations receive observable implementation + params in `eval_batch`
  and build the batch observable state internally.
- Sampler-aggregator engines produce one batch per call (`produce_batch`); the runner owns
  per-tick multi-batch production loops and queue-capacity limiting.
- Sampler-aggregator engines may return optional local in-memory batch context
  (`BatchContext`) from `produce_batch`; the runner stores it keyed by `batch_id`
  and passes it back to `ingest_training_weights`.
- Batch context is process-local only (not persisted to DB); implementations must
  tolerate missing context after restart.
- Runs specify evaluator/sampler/observable implementations independently.
- Evaluator/sampler implementation names remain in `integration_params`; observable implementation is in `runs.observable_implementation`.
- Concrete engine implementations should parse JSON params through `engines::BuildFromJson` (typed params + validation) instead of ad-hoc per-engine parsing helpers.
- Keep compatibility rules in typed implementation enums and validate at
  startup (for evaluator/observable: `EvaluatorImplementation::supports_observable`).
- `scalar` observable tracks `count`, `sum_weight`, `sum_abs`, and `sum_sq` over evaluator values.
- Observable payload handling should use serde-derived structs (`Serialize`/`Deserialize`) plus
  `Observable::{load_state_from_json, merge_state_from_json}`; avoid manual `json!`
  object construction and field-by-field `Value::get` merging in observable implementations.
- Completed batches are consumed by sampler-aggregator and deleted from `batches`; there is no persisted sampler engine state checkpoint.
- `run_node` role is controlled by DB desired assignments for its `node_id`; CLI does not select role.
- A `run_node` process executes at most one active role task at a time.
- `run_node` execution model is supervisor + worker task: outer poll loop reconciles desired assignment and starts/stops one spawned role task for the current run.
- Worker runtime state is encapsulated in `src/runners/node_runner.rs` (`NodeRunner`/`ActiveWorker`) and owns its store handle.
- Keep role switching safe: stop old role task, then start new one.
- Worker registration metadata uses run-spec implementation strings and binary
  version (`CARGO_PKG_VERSION`).
- `run_node` initializes tracing with DB persistence enabled; only events with
  `target="worker_log"` are written to `worker_logs`.
- Log events intended for dashboard visibility should include at least:
  `run_id`, `worker_id`, `role`, `event_type`.
- Read API includes `GET /api/runs/:id/logs` (optional `limit`, `worker_id`, `level`).

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
