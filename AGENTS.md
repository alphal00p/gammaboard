# AGENTS

## Purpose
This file is for contributors making structural or behavioral changes.
Use `README.md` for operator onboarding. Keep this file focused on architecture, invariants, and implementation rules.

## System Snapshot
- `gammaboard run` manages run creation, pause, and removal.
- `gammaboard node` manages desired assignments.
- `gammaboard run-node` reconciles DB desired assignment into at most one active local role loop.
- `gammaboard server` exposes the dashboard read API.
- PostgreSQL is the source of truth for runs, batches, node state, logs, and snapshots.

## Module Ownership
- `src/core/*`: domain types and store-facing contracts.
- `src/engines/*`: engine traits, configs, implementations, and shared run-spec wiring.
- `src/runners/*`: evaluator, sampler-aggregator, and node orchestration loops.
- `src/stores/*`: PostgreSQL implementation, queries, read DTOs, and bootstrap/run-control store composition.
  - Includes run-control composition traits used by runners (for example `RunControlStore`).
- `src/server/*`: dashboard read API runtime and route handlers.
- `src/tracing.rs`: tracing initialization and DB log sink wiring.
- `src/cli/*` and `src/main.rs`: CLI argument/wiring and process bootstrap.

## Operational Conventions
- Run config is TOML via `gammaboard run add <file.toml>`.
- `run add` deep-merges `configs/default.toml` with the provided file.
- Preflight derives `point_spec` from the evaluator and performs a one-point sampler -> parametrization -> evaluator dry-run before persistence.
- `run add` also persists evaluator and sampler init metadata.
- Point dimensions are canonical in `runs.point_spec`; do not duplicate them outside evaluator config unless the evaluator intrinsically needs them.
- `run pause`, `run remove`, and `node stop` support positional IDs or `-a/--all`.
- `gammaboard node list` is the node inventory view; it should print one row per node with `ID / Run / Role / Last Seen`. Inactive nodes should show `Run = N/A` and `Role = None`.
- Desired assignment is node-level: each node may have at most one desired role/run assignment at a time, and `node assign` should replace any existing desired assignment on that node.
- `node unassign` should clear the node's desired assignment without requiring a role.
- Desired assignments may include many evaluators per run, but must allow at most one sampler-aggregator per run. Enforce that invariant in the database and surface a clear CLI/store error on violation.
- Current assignments may include many evaluators per run, but must allow at most one current sampler-aggregator per run. Enforce that invariant in the database. Recovery from stale current sampler state is manual for now.
- Local worker bootstrapping for manual/live-test flows should go through `just start <N>` so worker IDs stay sequential as `w-1` through `w-N`.
- `gammaboard completion <shell>` should emit shell completion scripts to stdout; local build workflow also provides `~/.cargo/bin/gammaboard` as a symlink to the built binary so the latest local build can replace a Cargo-installed command in place.
- Run pause is implemented by clearing desired assignments so `run-node` reconciles down cleanly.
- Run lifecycle is derived from control-plane state; do not reintroduce a persisted `runs.status` column unless explicitly requested.
- Auto-stop conditions in `sampler_aggregator_runner_params.stop_on` are evaluated against aggregated observable samples; once reached, stop new production and clear desired assignments for the run.
- Sampler pause/resume snapshots are persisted on `runs.sampler_runner_snapshot`; keep the persisted shape explicit and versioned.
- The current snapshot restore path is implementation-specific; adding a new sampler requires adding snapshot export/restore support for it.
- Do not expose `runs.sampler_runner_snapshot` through the read API or dashboard payloads; it is internal resumability state and may be large.
- `run-node` must stop the old role before starting a new one.
- Role start failures are capped per desired target; after the cap is hit, retries stay disabled until desired assignment changes.
- Nodes register and heartbeat through the `nodes` table even when idle so node inventory is visible before any role assignment.
- Desired and current node assignments live directly on `nodes`, with `(desired_run_id, desired_role)` and `(active_run_id, active_role)` required to be both null or both set.
- Node shutdown is a one-shot signal read from `nodes.shutdown_requested_at`.
- Local serve contract:
  - backend port env var is `GAMMABOARD_BACKEND_PORT`
  - frontend API base URL is `REACT_APP_API_BASE_URL`
  - backend and frontend are started separately
  - frontend worker polling is shared app-wide; avoid adding duplicate per-panel polls for the same resource
- Local DB startup contract:
  - `just start-db` must wait for Compose health before migrations
  - Compose host port binding must use `DB_PORT` so local Postgres conflicts can be resolved in `.env`
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
- Sampler-aggregators own any per-batch training correlation state internally; do not pass runner-managed batch context back into them.
- The latest full aggregated observable should live on `runs.current_observable`.
- Snapshot persistence should use each observable's reduced persistent payload, not the tagged runtime enum form.
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
- Evaluator performance history should stay generic-metrics-only; evaluator-specific diagnostics do not belong there and static evaluator details belong in init metadata.
- Sampler performance history may include generic sampler metrics and sampler runtime metrics, but sampler-specific diagnostics should remain separate from generic metrics.
- Snapshot rows are point-in-time; do not reintroduce window semantics.
- Read APIs should serialize `BIGINT` IDs as strings.
- Run read payloads should expose `runs.point_spec` as `point_spec`.
- Frontend run views should prefer persisted run/history state for finished runs; live worker state is only for currently active telemetry.
- The `Workers` tab is for live worker registry state (assignment, heartbeat, role); historical performance belongs in a separate run+worker performance view.
- The `Performance` tab should stay run-scoped and worker-scoped over persisted snapshot history; sampler produce and ingest timing should remain distinct views rather than a merged single timing metric.
- In the `Runs` tab, sampler summary panels should emphasize targets, current runtime values, and current performance metrics. Low-signal tuning bounds belong in the JSON/config view rather than the primary summary card.
- Run log reads should stay server-filtered and cursor-paged (`limit`, `worker_id`, `level`, `q`, `before_id`) with response `{ items, next_before_id, has_more_older }`.
- The dashboard log viewer is view-only; prefer backend-driven pagination/filtering over rich client grid state.

## Schema Policy
- No backward-compat requirement by default.
- Prefer direct current-schema migrations unless compatibility work is explicitly requested.

## Required Checks
- `cargo fmt`
- `cargo check -q`
- `cargo test -q`
- `just test-e2e` for the ignored full-stack CLI flow when validating end-to-end behavior against a real local Postgres instance

## Documentation Rule
If you change structure, operations, CLI behavior, config schema, or runtime behavior, update `README.md` and `AGENTS.md` in the same change.
