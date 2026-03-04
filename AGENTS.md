# AGENTS

## Purpose
This file is for coding agents and contributors making structural or behavioral changes.
Use `README.md` for operator onboarding. Keep this file focused on internal architecture and implementation rules.

## System Snapshot
- `gammaboard run` and `gammaboard node` manage runs and desired assignments.
- `gammaboard run-node` is role-agnostic and reconciles DB desired assignment into one active local role task.
- `gammaboard server` exposes read APIs and SSE for the dashboard.
- PostgreSQL is the source of truth for run state, queue state, leases, worker state, logs, and snapshots.

## Module Ownership
- `src/core/*`: shared domain models and store-facing contracts (`Batch`, `BatchResult`, `PointSpec`, `RunStatus`, store traits, `StoreError`).
- `src/engines/*`: engine traits, factories, implementations, and shared run-spec wiring.
  - Role modules: `engines::evaluator`, `engines::sampler_aggregator`, `engines::observable`, `engines::parametrization`.
  - Shared types: `engines::shared::{BuildFromJson, IntegrationParams, RunSpec}`.
- `src/runners/*`: orchestration loops (`NodeRunner`, evaluator runner, sampler-aggregator runner).
- `src/stores/*`: PostgreSQL implementation and read DTOs/traits.
- `src/telemetry.rs`: tracing setup and runtime-log sink wiring through `core::RuntimeLogStore`.
- `src/main.rs` + `src/cli/*`: operational CLI entrypoint and subcommand modules (`run`, `node`, `run-node`, `server`).

## Operational Conventions
- Run config is TOML via `gammaboard run add <file.toml>`.
- `run-add` deep-merges `configs/default.toml` with the provided file (provided file wins).
- Keep split local live-test recipes in `justfile`:
  - `live-test-basic` for unit-line + Symbolica 3D Gaussian scenarios.
  - `live-test-gammaloop` for GammaLoop-only scenario.
- Runtime defaults live in `configs/default.toml`, not Rust `Default` impls.
- Run identity uses top-level `name` and persists to `runs.name`.
- Optional top-level `target` is persisted verbatim in `runs.target`; backend does not interpret it.
- Run lifecycle status persists in `runs.status` and is controlled by `gammaboard run` commands.
- `run start`/`run pause`/`run stop`/`run remove` and `node stop` use shared selectors: `-a|--all` or positional IDs.
- `run pause`/`run stop` must also clear desired assignments so run-nodes reconcile down cleanly.
- `run_node` executes at most one active role task at a time.
- Role switching must be stop-old-then-start-new.
- Node shutdown requests are consumed from `workers.shutdown_requested_at` as one-shot signals.
- Server/default local serve port contract:
  - Use env var `GAMMABOOARD_BACKEND_PORT` for backend port.
  - `just serve-backend` and `just serve-frontend` source `.env` and use the same backend port.
  - There is no combined `just serve` orchestrator; run backend/frontend in separate terminals.
  - Each `serve-*` command should stop its own previously running process before starting.
  - Frontend API URL is provided through `REACT_APP_API_BASE_URL`.
- Local DB startup contract:
  - `just start-db` must wait for Compose health (`docker-compose up -d --wait`)
    before running `sqlx migrate run`.
  - Postgres service in `docker-compose.yml` must keep a valid healthcheck so
    startup is deterministic.
  - `gammaboard` DB pool initialization includes retry/backoff for transient
    connect errors; keep retries in Rust startup path, not shell wrappers.

## Engine and Data Rules
- Keep runtime construction factory-based (`EvaluatorFactory`, `SamplerAggregatorFactory`, `ParametrizationFactory`, `ObservableFactory`) returning boxed trait objects.
- Avoid runtime `*Engine` dispatch enums.
- Parse engine params through `BuildFromJson` typed params + validation.
- `IntegrationParams`/`RunSpec` runner params are strongly typed (`EvaluatorRunnerParams`, `SamplerAggregatorRunnerParams`).
- Evaluators are batch-oriented (`Batch -> BatchResult`) and must support `batches.requires_training` semantics.
- Observable construction/ingestion is capability-based (`as_scalar_ingest`, `as_complex_ingest`).
- Observable aggregation is observable-to-observable merge.
- Batch payloads in `batches.points` must remain compact and shape-stable:
  - row-major flat `continuous`/`discrete`,
  - per-sample `weights`,
  - explicit 2D shape metadata.
- Completed batches are consumed by sampler-aggregator and deleted from `batches`.

## Performance, Logs, and Read APIs
- Worker performance history is persisted in:
  - `evaluator_performance_history`,
  - `sampler_aggregator_performance_history`.
- Snapshot rows are point-in-time (`created_at`); do not rely on window columns.
- Latest worker stats in `/api/workers` come from role-specific latest views.
- Runtime logs persist to `runtime_logs` based on tracing context (`source`, `run_id`, `worker_id`) and DB sink policy.
- Set `GAMMABOARD_DISABLE_DB_LOGS=1` to disable runtime-log DB persistence for CLI processes while keeping console tracing enabled.
- `engine` is tracing context only (used for sink-level filtering) and is not persisted as a `runtime_logs` column.
- SQL for runtime-log persistence lives in the Pg store query layer (`stores::queries::runtime_logs`);
  tracing should not issue raw SQL directly.
- Worker dashboard logs are read as `source='worker'` from `runtime_logs`; include
  `run_id` and `worker_id` when available.
- Read APIs include:
  - `GET /api/runs/:id/logs`,
  - `GET /api/workers`,
  - `GET /api/runs` / `GET /api/runs/:id` (includes opaque `target` payload when present),
  - `GET /api/runs/:id/performance/evaluator`,
  - `GET /api/runs/:id/performance/sampler-aggregator`,
  - `GET /api/runs/:id/stream` (shared per-run polling backend task, fanout broadcast).

## Schema and Migration Policy
- No backward-compat requirement by default.
- Prefer direct current-schema migrations over compatibility `ALTER TABLE` chains unless explicitly requested.

## Required Checks Before Finishing
- `cargo fmt`
- `cargo check -q`
- `cargo test -q`

## Documentation Sync Rule
If a change affects structure or operations, update docs in the same change:
- Always update `AGENTS.md` for internal conventions.
- Always update `README.md` for user-facing workflows/commands.

Changes that require doc updates include:
- module moves/renames,
- binary/CLI changes,
- config schema changes,
- migration/schema behavior changes,
- run orchestration/node assignment behavior changes.
