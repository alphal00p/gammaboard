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
- Node lease renewal must run independently from the main reconcile/tick loop so long role ticks cannot starve announces.
- If announce fails for 30 seconds, the node shuts down.
- `node run` reconcile polling should use a fast-start backoff: start at `50ms`, multiply by `2.0`, cap at `2s`, and reset on meaningful role/task changes.
- `node run` reconcile backoff should add bounded jitter around the exponential sleep to reduce synchronized retries between workers.
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
- There is no reserved `init` run task; initialization is represented only by the root stage snapshot.
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
- `sample` with `nr_samples = 0` is the only supported no-work stage update task shape, including pure configuration updates.
- GammaLoop evaluator point dimensions are inferred from the selected integrand. Do not configure `continuous_dims` or `discrete_dims` for `evaluator.kind = "gammaloop"`.
- `observable = { config = "gammaloop" }` is supported only with `evaluator.kind = "gammaloop"` and persists GammaLoop's native histogram snapshot bundle directly.
- Persisted and API-facing observable payloads must remain JSON-safe. Observable implementations must not emit raw `NaN` or infinite `f64` values into serialized state; they must sanitize, summarize, count, or reject such values explicitly inside the observable implementation instead of relying on storage-layer serialization failures. Full observables must preserve positional cardinality when non-finite values occur and persist which entry positions were invalid instead of dropping them.
- Task files used for `run task add` may contain either `task = { ... }`, `[[task_queue]]`, or both. Normalize them as `task` first, then `task_queue`. Missing both should resolve to an empty task list.
- There is no run-level observable default. A first executable task that needs a fresh observable must declare it explicitly.
- `image` and `plot_line` tasks must declare their observable family explicitly and start with a fresh full observable.
- Fresh sampler tasks may inherit a reduced initial batch size from the previous sampler task, but should not carry over the full rolling metrics state.
- Claimed batches are fenced by live node ownership. Do not add a second independent batch lease.
- Whether evaluator training values are required is a per-batch persisted contract (`batches.requires_training_values`), not inferred at ingest time from the currently active sampler config.
- Queue payloads are transient and may use compact binary storage; do not optimize them for ad hoc SQL readability at the expense of runtime throughput.
- Havana training and inference samplers must support nested discrete domains and preserve the full grid topology in persisted snapshots for restore/materialization.
- Havana training runs in deterministic lockstep windows: it emits at most one `samples_for_update` window at a time, pauses production until that window is fully ingested, then updates the grid and continues.
- Sample tasks must force an initial small batch round-trip before normal queue ramp-up so an observable snapshot is persisted immediately at task start, and must persist the observable again when the task completes.
- Sampler-aggregator completed-batch ingestion should advance from a persisted `batch.id` cursor, not rescan the whole run on every tick.
- Sampler-aggregator hot-loop control should reuse queue snapshots where possible and prefer direct evaluator counts over materializing full node rows.
- `sampler_aggregator_runner_params.frontend_sync_interval_ms` controls how often frontend-facing observable state is refreshed during sampling; full sampler resume checkpoints are persisted only on unassignment/pause, and task completion still forces a final observable flush.
- Evaluators use a fixed single-slot latent prefetch and single-slot async submit pipeline to hide DB latency. Materialization and evaluation remain strictly one batch at a time.
- Evaluators are stateless across reconcile-down. On stop they should drain already-claimed local latent batches without claiming new work, not persist evaluator state.
- `sampler_aggregator_runner_params.queue_buffer` is the single public queue buffer control. The runner targets about `queue_buffer * active_evaluator_count` pending batches. `0.0` is the most aggressive setting and lets pending work drain to zero when the sampler cannot refill fast enough; larger values keep more pending work buffered. `max_queue_size` remains the hard cap.
- `sampler_aggregator_runner_params.strict_batch_ordering` controls whether completed evaluator batches are ingested strictly as a contiguous id prefix or opportunistically in completed-id order.
- Sampler frontend sync is lightweight and periodic: it updates `runs.current_observable`, appends `persisted_observable_snapshots`, and records performance snapshots. Full sampler resume state belongs in `run_sampler_checkpoints`, which is overwritten on unassignment/pause and contains the full sampler-aggregator checkpoint blob.

## Panels And Dashboard
- Backend visualization uses the generic panel model in `src/server/panels.rs`.
- The frontend should render panels generically; it should not reimplement domain projections or panel merge semantics.
- GammaLoop sample observables should project a histogram bundle table whose payload includes the histogram bins; the frontend renders the selected-histogram chart client-side as a stepped histogram with bin error bars and a linear/log y-scale toggle, and table row selection should drive the selected histogram using the live bundle rows.
- Panel APIs are poll-based: clients send an optional opaque `cursor`, plus `panel_state` and `panel_actions`; the backend returns `PanelResponse`.
- `append` is only valid when the backend can safely extend existing state; otherwise it must send `replace`.
- Panel specs may include simple width hints such as `compact`, `half`, and `full`.
- Run info, task output, worker details, performance, and engine config should stay backend-owned.
- Dashboard auth is operator-oriented: read-only endpoints may stay open, while explicit steering endpoints require admin auth.
- Dashboard steering should use explicit endpoints such as `pause`, `assign`, `unassign`, `append task`, `remove pending task`, `create run`, `clone run`, and `remove run`, not generic patch endpoints.
- Dashboard auth is intended for small trusted deployments behind HTTPS.
- Run and task templates should be simple `.toml` files served from server-configured directories; the frontend should treat them as editable starting points, not as a second schema.
- Shared runtime database and tracing settings should come from `configs/runtime/default.toml` by default, with an optional global `--runtime-config <PATH>` override.
- Local Postgres lifecycle commands should live under `gammaboard db ...` and use the shared runtime config instead of separate env-driven just recipes; `just db-reset` may wrap `gammaboard db stop`, `gammaboard db delete --yes`, and `gammaboard db start` for convenience.
- Local Postgres tuning lives under `runtime.local_postgres`; keep latency-sensitive queue defaults explicit there, including WAL/checkpoint settings and whether `synchronous_commit` is relaxed for local throughput.
- Server API bind, allowed origins, secure cookie policy, `allow_db_admin` policy, dashboard auth secrets, and template directories should come from `configs/server/default.toml` by default, with an optional `gammaboard server --server-config <PATH>` override.
- Detached deploy lifecycle should live under `gammaboard deploy ...`, with `configs/deploy/*.toml` owning frontend HTTP exposure, static-site serving, and cleanup policy, while selecting which `configs/server/*.toml` backend profile to run.
- `gammaboard server` remains the direct foreground backend path; `gammaboard deploy` is orchestration around it, not a replacement for it.
- Server TOML should be explicit; do not rely on implicit defaults for required server settings.
- `gammaboard server` should terminate immediately on `Ctrl-C` (no graceful-drain wait path).

## Logging And Read APIs
- Runtime logs are persisted through the tracing pipeline into PostgreSQL.
- Worker performance history is append-only, and dashboard "latest" performance reads should come from dedicated latest tables maintained on write, not recomputed latest-per-worker views over history.
- The global `/nodes` list is a lightweight summary read; do not join per-worker metrics into the hot polling list query. Load worker metrics/details only for focused views.
- Run read APIs should keep `batches` as the source of truth; when batch aggregation is expensive, prefer scoped multi-query reads over duplicated persisted queue counters.
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
