# AGENTS

Use this file for architecture and implementation rules. Use `README.md` for setup and normal usage.

## Ownership
- `src/api/*`: high-level typed application use-cases shared by CLI and server.
- `src/core/*`: shared contracts, run/task types, store traits, errors.
- `src/evaluation/*`: evaluator-side batch/result semantics and observables.
- `src/sampling/*`: sampler-side latent queue semantics, samplers, materializers, and batch transforms.
- `src/runners/*`: evaluator/sampler runtimes and node reconciliation loops.
- `src/stores/*`: PostgreSQL store, queries, read models.
- `src/server/*`: dashboard API and backend panel projection.
- `src/cli/*`: CLI parsing and process bootstrap.

## Core Rules
- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, and snapshots.
- Concrete evaluator batches are `Vec<Point>`, not rectangular matrices.
- Run-global layout metadata uses `Domain`, not `PointSpec`.
- Runs are driven by persisted `run_tasks`. The evaluator work queue is lower-level and distinct.
- `RunSpec` should keep only immutable run-global state. Task-varying sampler, materializer, batch-transform, and observable choices belong on tasks or in stored integration defaults, not on `RunSpec`.
- Run names are human-facing and not unique. CLI run references may be numeric ids or exact names; ambiguous names must fail and print matches.
- If `task_queue` is omitted during `run add`, the run is created idle.
- `run add` must persist an initial queue-empty `run_stage_snapshot` immediately.
- Run lifecycle is derived from control-plane state. Do not add a persisted run status column unless explicitly requested.
- Pausing a run means clearing desired assignments so `node run` workers reconcile down cleanly.

## Nodes
- Node identity is split into `nodes.name` and `nodes.uuid`.
- `name` is the unique operator-facing handle.
- `uuid` is the live `node run` process incarnation.
- Nodes use a single announce operation to register and renew their lease.
- If announce fails for 30 seconds, the node shuts down.
- `node run` reconcile polling should use a fast-start backoff: start at `50ms`, multiply by `2.0`, cap at `2s`, and reset on meaningful role/task changes.
- `node run` should terminate immediately on `Ctrl-C` and `SIGTERM`.
- Graceful shutdown should expire the lease immediately so the same node name can be reused at once.
- Desired/current assignments live directly on `nodes`.
- At most one sampler-aggregator may be assigned to a run at a time. Many evaluators are allowed.

## Tasks, Snapshots, Queue
- Task sequencing lives in `src/core/tasks.rs`.
- Task ordering may still use internal sequence numbers, but operator-facing task identity is the task `name`.
- Task names must be unique per run.
- Task names are optional in task TOML. If omitted, the system auto-generates a stable default name.
- Use a single task shape (`RunTaskSpec`) end-to-end; do not reintroduce separate task input/spec indirection.
- Task transitions must restore runtime state from persisted `run_stage_snapshots`, not in-memory handoff only.
- Snapshots are the branchable state timeline. Tasks are queued work items that may produce snapshots, but are not themselves the canonical branch identity.
- Every run must persist a root stage snapshot at initialization with `sequence_nr = 0` and `task_id = null`.
- Every run must also persist a completed reserved `init` run task at `sequence_nr = 0` so initialization is visible in task lists.
- `init` is system-reserved and must not be accepted from user task TOML.
- Sample source selection is per component (`sampler_aggregator`, `observable`), not a task-level snapshot id.
- Cloning a run from a stage snapshot must not copy queued tasks; the cloned run starts idle at that cloned root snapshot.
- A cloned run root snapshot name should identify the source run and source task (or root) it was cloned from.
- Sample source specs support three forms: omitted/`"latest"`, `{ from_name = "<task-name>" }`, or `{ config = ... }`.
- Omitted source fields must resolve as latest; no legacy snapshot-id fallback is allowed.
- Task preflight belongs on task insertion. Bare `run add` should validate run-global construction and root-stage creation, while appended tasks should be validated against the current or referenced stage snapshots before persistence.
- Shared run and node orchestration should live in `src/api/*`; CLI and server should stay thin adapters around typed API calls.
- Task `batch_transforms` is stage state: omitted inherits the previous effective stage, and `batch_transforms = []` explicitly clears inherited transforms.
- Sample tasks may omit `sampler_aggregator`; omitted sampler uses the previous effective stage.
- Sample tasks may omit `observable`; that means reuse the previous observable state.
- Havana inference source selection lives inside Havana sampler config. Default is `latest_training_sampler_aggregator`, with optional explicit `snapshot_id`.
- `sample` with `nr_samples = 0` is the only supported no-work stage update task shape.
- GammaLoop evaluator point dimensions are inferred from the selected integrand. Do not configure `continuous_dims` or `discrete_dims` for `evaluator.kind = "gammaloop"`.
- Task files used for `run task add` may contain either `task = { ... }`, `[[task_queue]]`, or both. Normalize them as `task` first, then `task_queue`. Missing both should resolve to an empty task list.
- There is no run-level observable default. A first executable task that needs a fresh observable must declare it explicitly.
- `image` and `plot_line` tasks must declare their observable family explicitly and start with a fresh full observable.
- Fresh sampler tasks may inherit a reduced initial batch size from the previous sampler task, but should not carry over the full rolling metrics state.
- Claimed batches are fenced by live node ownership. Do not add a second independent batch lease.
- Whether evaluator training values are required is a per-batch persisted contract (`batches.requires_training_values`), not inferred at ingest time from the currently active sampler config.

## Panels And Dashboard
- Backend visualization uses the generic panel model in `src/server/panels.rs`.
- The frontend should render panels generically; it should not reimplement domain projections or panel merge semantics.
- Panel APIs are poll-based: clients send an optional opaque `cursor`, plus `panel_state` and `panel_actions`; the backend returns `PanelResponse`.
- `append` is only valid when the backend can safely extend existing state; otherwise it must send `replace`.
- Panel specs may include simple width hints such as `compact`, `half`, and `full`.
- Run info, task output, worker details, performance, and engine config should stay backend-owned.
- Dashboard auth is operator-oriented: read-only endpoints may stay open, while explicit steering endpoints require admin auth.
- Dashboard steering should use explicit endpoints such as `pause`, `assign`, `unassign`, `append task`, `remove pending task`, `create run`, `clone run`, and `remove run`, not generic patch endpoints.
- Dashboard auth is intended for small trusted deployments behind HTTPS.
- Run and task templates should be simple `.toml` files served from server-configured directories; the frontend should treat them as editable starting points, not as a second schema.
- Shared CLI database and tracing settings should come from `configs/cli/default.toml` by default, with an optional global `--cli-config <PATH>` override.
- Local Postgres lifecycle commands should live under `gammaboard db ...` and use the shared CLI config instead of separate env-driven just recipes.
- Server host, port, allowed origin, secure cookie policy, `allow_db_admin` policy, dashboard auth secrets, and template directories should come from `configs/server/default.toml` by default, with an optional `gammaboard server --server-config <PATH>` override.
- Server TOML should be explicit; do not rely on implicit defaults for required server settings.
- `gammaboard server` should terminate immediately on `Ctrl-C` (no graceful-drain wait path).

## Logging And Read APIs
- Runtime logs are persisted through the tracing pipeline into PostgreSQL.
- Log read APIs should expose `node_name` and `node_uuid` even if SQL columns still use older names.
- Read APIs should serialize `BIGINT` ids as strings.

## Runtime Metadata
- Do not persist evaluator/sampler init metadata on runs or expose it in run panels/apis unless explicitly requested.
- `unit` evaluator and `naive_monte_carlo` sampler include optional fault-injection parameters for e2e testing (`fail_on_batch_nr`, `fail_on_produce_batch_nr`, `fail_on_materialize_batch_nr`); keep them optional and default-off.

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
