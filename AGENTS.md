# AGENTS

Use this file for architecture and implementation rules. Use `README.md` for setup and normal usage.

## Ownership
- `src/core/*`: shared contracts, run/task types, store traits, errors.
- `src/evaluation/*`: evaluator-side batch/result semantics and observables.
- `src/sampling/*`: sampler-side latent queue semantics, samplers, parametrizations.
- `src/runners/*`: evaluator/sampler runtimes and node reconciliation loops.
- `src/stores/*`: PostgreSQL store, queries, read models.
- `src/server/*`: dashboard API and backend panel projection.
- `src/cli/*`: CLI parsing and process bootstrap.

## Core Rules
- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, and snapshots.
- Runs are driven by persisted `run_tasks`. The evaluator work queue is lower-level and distinct.
- `RunSpec` should keep only immutable run-global state. Task-varying sampler, parametrization, and observable choices belong on tasks or in stored integration defaults, not on `RunSpec`.
- Run names are human-facing and not unique. CLI run references may be numeric ids or exact names; ambiguous names must fail and print matches.
- If `task_queue` is omitted during `run add`, the run is created idle.
- Run lifecycle is derived from control-plane state. Do not add a persisted run status column unless explicitly requested.
- Pausing a run means clearing desired assignments so `run-node` reconciles down cleanly.

## Nodes
- Node identity is split into `nodes.name` and `nodes.uuid`.
- `name` is the unique operator-facing handle.
- `uuid` is the live `run-node` process incarnation.
- Nodes use a single announce operation to register and renew their lease.
- If announce fails for 30 seconds, the node shuts down.
- `run-node` reconcile polling should use a fast-start backoff: start at `100ms`, multiply by `1.1`, cap at `1s`, and reset on meaningful role/task changes.
- Graceful shutdown should expire the lease immediately so the same node name can be reused at once.
- Desired/current assignments live directly on `nodes`.
- At most one sampler-aggregator may be assigned to a run at a time. Many evaluators are allowed.

## Tasks, Snapshots, Queue
- Task sequencing lives in `src/core/tasks.rs`.
- Task transitions must restore runtime state from persisted queue-empty `run_stage_snapshots`, not in-memory handoff only.
- Executable tasks may declare `start_from = { run_id, task_id }` to branch from an older task snapshot.
- `configure` tasks update sampler, parametrization, and observable state without producing work; omitted fields inherit the previous effective stage.
- Sample and `configure` tasks may omit `observable`; that means reuse the previous observable state.
- There is no run-level observable default. A first executable task that needs a fresh observable must declare it explicitly.
- `image` and `plot_line` tasks must declare their observable family explicitly and start with a fresh full observable.
- Fresh sampler tasks may inherit a reduced initial batch size from the previous sampler task, but should not carry over the full rolling metrics state.
- Claimed batches are fenced by live node ownership. Do not add a second independent batch lease.

## Panels And Dashboard
- Backend visualization uses the generic panel model in `src/server/panels.rs`.
- The frontend should render panels generically; it should not reimplement domain projections or panel merge semantics.
- Panel APIs are poll-based: clients send an optional opaque `cursor`, plus `panel_state` and `panel_actions`; the backend returns `PanelResponse`.
- `append` is only valid when the backend can safely extend existing state; otherwise it must send `replace`.
- Panel specs may include simple width hints such as `compact`, `half`, and `full`.
- Run info, task output, worker details, performance, and engine config should stay backend-owned.
- Dashboard auth is operator-oriented: read-only endpoints may stay open, while explicit steering endpoints require admin auth.
- Dashboard steering should use explicit endpoints such as `pause`, `assign`, `unassign`, `append task`, `create run`, and `clone run`, not generic patch endpoints.
- Dashboard auth is intended for small trusted deployments behind HTTPS.
- Run and task templates should be simple `.toml` files served from server-configured directories; the frontend should treat them as editable starting points, not as a second schema.
- Shared CLI database and tracing settings should come from `configs/gammaboard.toml` by default, with an optional global `--cli-config <PATH>` override.
- Local Postgres lifecycle commands should live under `gammaboard db ...` and use the shared CLI config instead of separate env-driven just recipes.
- Server host, port, allowed origin, secure cookie policy, dashboard auth secrets, and template directories should come from `configs/server.toml` by default, with an optional `gammaboard server --server-config <PATH>` override.
- Server TOML should be explicit; do not rely on implicit defaults for required server settings.

## Logging And Read APIs
- Runtime logs are persisted through the tracing pipeline into PostgreSQL.
- Log read APIs should expose `node_name` and `node_uuid` even if SQL columns still use older names.
- Read APIs should serialize `BIGINT` ids as strings.

## Schema Policy
- No backward-compat requirement by default.
- Prefer direct current-schema migrations unless compatibility is explicitly requested.

## Required Checks
- `cargo fmt`
- `cargo check -q`
- `cargo test -q`
- `just test-e2e`

## Documentation Rule
- If you change architecture, runtime behavior, CLI behavior, or config shape, update this file.
- If you change normal setup or operator workflow, update `README.md` too.

## Commit Discipline
- Prefer small coherent stages.
- After a stage is ready, provide a concrete commit message as a bare fenced `text` block with no label or commentary around it.
