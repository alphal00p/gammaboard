# AGENTS

## Purpose
This file is for coding agents and contributors making structural or behavioral changes.
Use `README.md` for human/operator onboarding, and use this file for repo-internal rules.

## System Snapshot
- `control_plane` manages runs and desired role assignments.
- `run_node` runs on a node and reconciles desired roles into local tasks.
- `server` exposes read APIs and SSE for the dashboard.
- PostgreSQL is the source of truth for queue, control-plane state, leases, and snapshots.

## Module Ownership
- `src/core/*`
  - Shared domain and store-facing contracts.
  - `RunStatus`, worker/assignment models, store traits, `StoreError`.
- `src/engines/*`
  - Runtime engine contracts and implementations.
  - Engine implementation enums, run spec/integration params parsing, engine errors.
- `src/runners/*`
  - Orchestration loops (`NodeRunner`, evaluator runner, sampler-aggregator runner).
- `src/stores/*`
  - Postgres implementation and read-side DTOs/traits.
- `src/bin/*`
  - Operational binaries only (`control_plane`, `run_node`, `server`).

## Operational Conventions
- Run configuration is passed as TOML to `control_plane run-add`.
- Engine/runner settings are persisted in `runs.integration_params`; point shape is persisted in `runs.point_spec`.
- Batch payloads in `batches.points` must stay compact and shape-stable:
  row-major flat `continuous`/`discrete` arrays + explicit 2D shape metadata.
- Evaluators operate batch-wise (`Batch -> BatchResult`), where `BatchResult` contains
  training `values: Vec<f64>` and one aggregated batch-level observable JSON.
- Runs specify evaluator/sampler/observable implementations independently via
  `integration_params` (`*_implementation` string fields).
- Keep compatibility rules in typed implementation enums and validate at
  startup (for evaluator/observable: `EvaluatorImplementation::supports_observable`).
- `scalar` observable tracks `count`, `sum`, `sum_abs`, and `sum_sq` over evaluator values.
- Observable payload handling should use serde-derived structs (`Serialize`/`Deserialize`) plus
  `Observable::{load_state_from_json, merge_state_from_json}`; avoid manual `json!`
  object construction and field-by-field `Value::get` merging in observable implementations.
- Completed batches are consumed by sampler-aggregator and deleted from `batches`; there is no persisted sampler engine state checkpoint.
- Nodes are generic: one `run_node` process can reconcile both roles for assigned runs.
- Keep role switching safe: stop old role task, then start new one.
- Worker registration metadata uses run-spec implementation strings and binary
  version (`CARGO_PKG_VERSION`).

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
