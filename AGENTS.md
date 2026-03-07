# AGENTS

## Purpose
This file is for contributors making structural or behavioral changes.
Use `README.md` for operator onboarding. Keep this file focused on architecture, invariants, and implementation rules.

## System Snapshot
- `gammaboard run` manages run lifecycle.
- `gammaboard node` manages desired assignments.
- `gammaboard run-node` reconciles DB desired assignment into at most one active local role loop.
- `gammaboard server` exposes the dashboard read API.
- PostgreSQL is the source of truth for runs, batches, assignments, workers, logs, and snapshots.

## Module Ownership
- `src/core/*`: domain types and store-facing contracts.
- `src/engines/*`: engine traits, configs, implementations, and shared run-spec wiring.
- `src/runners/*`: evaluator, sampler-aggregator, and node orchestration loops.
- `src/stores/*`: PostgreSQL implementation, queries, and read DTOs.
- `src/tracing.rs`: tracing initialization and DB log sink wiring.
- `src/cli/*` and `src/main.rs`: CLI entrypoints and command wiring.

## Operational Conventions
- Run config is TOML via `gammaboard run add <file.toml>`.
- `run add` deep-merges `configs/default.toml` with the provided file.
- Preflight derives `point_spec` from the evaluator and performs a one-point sampler -> parametrization -> evaluator dry-run before persistence.
- `run add` also persists evaluator and sampler init metadata.
- Point dimensions are canonical in `runs.point_spec`; do not duplicate them outside evaluator config unless the evaluator intrinsically needs them.
- `run start`, `run pause`, `run stop`, `run remove`, and `node stop` support positional IDs or `-a/--all`.
- `run pause` and `run stop` must clear desired assignments so `run-node` reconciles down cleanly.
- `run-node` must stop the old role before starting a new one.
- Role start failures are capped per desired target; after the cap is hit, retries stay disabled until desired assignment changes.
- Node shutdown is a one-shot signal read from `workers.shutdown_requested_at`.
- Local serve contract:
  - backend port env var is `GAMMABOARD_BACKEND_PORT`
  - frontend API base URL is `REACT_APP_API_BASE_URL`
  - backend and frontend are started separately
- Local DB startup contract:
  - `just start-db` must wait for Compose health before migrations
  - DB retry/backoff belongs in Rust startup, not shell wrappers

## Engine and Data Rules
- Keep runtime construction config-based through `EvaluatorConfig`, `SamplerAggregatorConfig`, and `ParametrizationConfig`.
- Avoid adding runtime engine dispatch enums for evaluator/sampler/parametrization selection.
- Parse engine params with `BuildFromJson`.
- `IntegrationParams` and `RunSpec` carry strongly typed runner params.
- Evaluators are batch-oriented and must respect `batches.requires_training`.
- Observable state is evaluator-owned and serialized as semantic `ObservableState`.
- If an evaluator supports multiple observable semantics, that choice belongs in evaluator config via `observable_kind`.
- Sampler-aggregator aggregation is `ObservableState` merge, not capability-style ingest.
- Batch payloads in `batches.points` must stay compact and shape-stable:
  - row-major flat `continuous` and `discrete`
  - per-sample `weights`
  - explicit 2D shape metadata
- Completed batches are consumed by sampler-aggregator and deleted.

## Logging and Read APIs
- Runtime logs are persisted from tracing context through `RuntimeLogStore`.
- SQL for runtime log persistence lives in the store/query layer, not in tracing setup.
- Runtime log context should include `source`, `run_id`, `worker_id`, and `node_id` when available.
- Set `GAMMABOARD_DISABLE_DB_LOGS=1` to disable DB log persistence.
- DB log thresholds are configured with:
  - `GAMMABOARD_DB_LOG_LEVEL`
  - `GAMMABOARD_DB_EXTERNAL_LOG_LEVEL`
- Worker performance history is stored in:
  - `evaluator_performance_history`
  - `sampler_aggregator_performance_history`
- Snapshot rows are point-in-time; do not reintroduce window semantics.
- Read APIs should serialize `BIGINT` IDs as strings.

## Schema Policy
- No backward-compat requirement by default.
- Prefer direct current-schema migrations unless compatibility work is explicitly requested.

## Required Checks
- `cargo fmt`
- `cargo check -q`
- `cargo test -q`

## Documentation Rule
If you change structure, operations, CLI behavior, config schema, or runtime behavior, update `README.md` and `AGENTS.md` in the same change.
