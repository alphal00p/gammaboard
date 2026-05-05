# AGENTS

Use this file for architecture and implementation rules. Use `README.md` for setup and normal usage.

## Ownership
- `src/api/*`: high-level typed application use-cases shared by CLI and server.
- `src/core/*`: shared contracts, run/task types, store traits, errors.
- `src/evaluation/*`: evaluator-side batch/result semantics and accumulators.
- `src/sampling/*`: sampler-side latent queue semantics, samplers, materializers, and batch transforms.
- `src/runners/*`: evaluator/sampler runtimes and node reconciliation loops.
- `src/stores/*`: PostgreSQL store, queries, read models.
- `src/server/*`: dashboard API and backend panel projection.
- `src/cli/*`: CLI parsing and process bootstrap.

## Core Rules
- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, and snapshots.
- Concrete evaluator batches are `Vec<Point>`, not rectangular matrices.
- Point weights are stored as named `weight_factors`; effective sample weight is the product of those factors (`total_weight`).
- Run-global layout metadata uses `Domain`, not `PointSpec`.
- Runs are driven by persisted `run_tasks`. The evaluator work queue is lower-level and distinct.
- `RunSpec` should keep only immutable run-global state. Task-varying sampler, materializer, batch-transform, and accumulator choices belong on tasks or in stored integration defaults, not on `RunSpec`.
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
- Expired node leases must not keep desired/current assignments alive. Assignment writes and replacement announces should opportunistically clear expired node assignments so stale sampler rows cannot block restart after an ungraceful control/database shutdown.
- Role tick pacing may differ by worker type. Keep evaluator polling conservative, but let sampler-aggregator ticks run more frequently so queue refill is not artificially bursty.
- `node run` should begin graceful shutdown immediately on `Ctrl-C` and `SIGTERM`.
- Graceful shutdown should expire the lease immediately so the same node name can be reused at once.
- Node shutdown requests clear desired assignments immediately and keep current assignments until the node reconciles down or its lease expires.
- Control shutdown goes through the shared graceful node shutdown API: clear all desired assignments first, request all nodes to stop, wait for active sampler-aggregator roles to persist/clear, then stop control-owned services.
- Desired/current assignments live directly on `nodes`.
- Node startup intent lives in `node_launch_requests`, not in `nodes`. A single launch request may represent many requested workers; resolver-specific details belong in its JSON args/result fields.
- Dashboard node-start actions should create launch requests. If `allow_local_node_spawn = true`, the control process may resolve those requests locally; otherwise an external launcher is expected to resolve them.
- External launchers must communicate through the dashboard API, not direct SQL. Launch request states are `pending`, `starting`, `running`, `failed`, and `canceled`; `starting` means jobs/processes were submitted, while `running` is reconciled from live node leases.
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
- Sample source selection is per component (`sampler_aggregator`, `accumulator`), not a task-level snapshot id.
- Cloning a run from a stage snapshot must not copy queued tasks; the cloned run starts idle at that cloned root snapshot.
- A cloned run root snapshot name should identify the source run and source task (or root) it was cloned from.
- Sample source specs support three forms: omitted/`"latest"`, `{ from_name = "<task-name>" }`, or `{ config = ... }`.
- Omitted source fields must resolve as latest; no legacy snapshot-id fallback is allowed.
- Task preflight belongs on task insertion. Bare `run add` should validate run-global construction and root-stage creation, while appended tasks should be validated against the current or referenced stage snapshots before persistence.
- Shared run and node orchestration should live in `src/api/*`; CLI and server should stay thin adapters around typed API calls.
- Task `batch_transforms` is stage state: omitted inherits the previous effective stage, and `batch_transforms = []` explicitly clears inherited transforms.
- Sample tasks may omit `sampler_aggregator`; omitted sampler uses the previous effective stage.
- Explicit sampler configs (`sampler_aggregator = { config = ... }`) start fresh and must not implicitly resume the previous sampler snapshot (except `havana_inference`, which still resolves its configured handoff source).
- Sample tasks may omit `accumulator`; that means reuse the previous accumulator state.
- Havana inference source selection lives inside Havana sampler config. Default is `latest_training_sampler_aggregator`, with optional explicit `snapshot_id`.
- `set_accumulator` is the explicit no-work task for changing accumulator state.
- `sample` tasks may omit `accumulator`; omitted means reuse the latest effective accumulator state, and task insertion must fail if none exists yet.
- Sample task stop conditions are configured through `stop_condition = { ... }`. Supported keys are `max_samples`, `absolute_error`, `relative_error`, and optional complex projection (`projection = "real" | "imag" | "abs"`).
- GammaLoop evaluator point dimensions are inferred from the selected integrand. Do not configure `continuous_dims` or `discrete_dims` for `evaluator.kind = "gammaloop"`.
- GammaLoop evaluation in Gammaboard uses x-space sampling. Do not enable momentum-space GammaLoop evaluation for normal runs; it bypasses the parameterized observable path used by native histograms.
- `evaluator.kind = "gammaloop"` supports optional `post_load_commands = ["set ...", ...]` that run after state load and before integrand selection. These commands must stay in-memory (no persisted state writes).
- Relative evaluator resource paths (for example GammaLoop `state_folder`) resolve against ordered `runtime.resources.roots`; first existing match wins and absolute paths are used as-is.
- `evaluator.kind = "python_scalar"` resolves the configured nix `flake_ref`, imports the configured `module` + `class`, and evaluates homogeneous fixed-rectangular batches via numpy-style `xs_discrete: (nr_samples, len(discrete_cardinalities))` plus `xs_continuous: (nr_samples, continuous_dims)` to `(nr_samples,)`. Discrete shape is configured explicitly by `discrete_cardinalities` (for example `[3, 4, 2]`) and validated axis-wise. Optional `init_args` are forwarded as a python dict to `from_config(discrete_cardinalities=..., continuous_dims=..., init_args=...)` when present, otherwise to `ClassName(**init_args)` / `ClassName()`.
- `sampler_aggregator.kind = "python_sampler"` resolves the configured nix `flake_ref`, imports the configured `module` + `class`, and produces homogeneous fixed-rectangular batches as `SampleBatch(xs_discrete, xs_continuous, weights)`. Domain must be fixed rectangular. Discrete shape is passed to Python as ordered `discrete_cardinalities` derived from the run domain; do not configure a separate Python sampler `discrete_dims` field. Returned `weights` are required and become the `sampler_weight` factor. Set `requires_training_values = true` for Python samplers that need evaluator feedback via `ingest_training_values`; otherwise batches are produced without training-value collection. Optional `init_args` are forwarded to python constructor paths (`from_snapshot`, then `from_config`, then kwargs constructor fallback). Optional Python-side `pdf` is vectorized the same way and receives both arrays.
- Python evaluator/sampler worker protocol scripts are checked in under `python_api/python_workers/` and should remain protocol-compatible when edited. Python-side contracts for user modules are split across `python_api/python_api/evaluator.py` and `python_api/python_api/sampler.py`.
- Samplers expose an optional `pdf(point)` hook where `point` is the materialized evaluator-domain point `(Vec<i64>, Vec<f64>)`; default is unsupported (`None`) for samplers that cannot define a meaningful PDF query.
- Homogeneous batch-evaluator helpers are for fixed-rectangular inputs with per-batch-constant discrete and continuous dimensions. They must reject mixed or ragged batch point layouts explicitly and keep output cardinality equal to `batch.size()`.
- `accumulator = { config = "gammaloop" }` is supported only with `evaluator.kind = "gammaloop"` and persists GammaLoop's native histogram snapshot bundle directly.
- GammaLoop accumulators also persist merged per-batch evaluation diagnostics (precision promotions, instability/NaN counters, and timing/event aggregates) alongside the histogram bundle so task panels can expose evaluator internals.
- Persisted and API-facing accumulator payloads must remain JSON-safe. Accumulator implementations must not emit raw `NaN` or infinite `f64` values into serialized state; they must sanitize, summarize, count, or reject such values explicitly inside the accumulator implementation instead of relying on storage-layer serialization failures. Full accumulators must preserve positional cardinality when non-finite values occur and persist which entry positions were invalid instead of dropping them.
- Task files used for `run task add` may contain either `task = { ... }`, `[[task_queue]]`, or both. Normalize them as `task` first, then `task_queue`. Missing both should resolve to an empty task list.
- There is no run-level accumulator default. A first executable task that needs a fresh accumulator must declare it explicitly.
- `accumulator = { config = "empty" }` is a valid no-op accumulator for tasks that need runtime/plumbing compatibility without accumulating accumulator state.
- `pdf_adaptation_image` is a dedicated task kind that rasterizes a plane with `pdf_adaptation_raster_plane`, defaults its sampler source to `latest`, and may also use `{ from_name = "<task-name>" }`.
- `pdf_adaptation_plot_line` is a dedicated task kind that rasterizes a line with `pdf_adaptation_raster_line`, defaults its sampler source to `latest`, and may also use `{ from_name = "<task-name>" }`.
- `pdf_adaptation_image` frontend snapshots come from sampler-owned persisted output payloads; `runs.current_observable` (current accumulator payload) stays the task's `empty` accumulator and is not the image data source.
- `pdf_adaptation_*` panels should treat `log(1)=0` as the neutral reference color on image plots (symmetric normalization), use plane-normalized logs for top `log_pdf`/`log_integrand` views, and expose both oversampling variants (`log(P / (|I|/<|I|>))` and `log((P/sum_plane P) / (|I|/sum_plane |I|))`).
- `image` and `plot_line` tasks must declare their accumulator family explicitly and start with a fresh full accumulator.
- Fresh sampler tasks may inherit a reduced initial batch size from the previous sampler task, but should not carry over the full rolling metrics state.
- Claimed batches are fenced by live node ownership. Do not add a second independent batch lease.
- Whether evaluator training values are required is a per-batch persisted contract (`batches.requires_training_values`), not inferred at ingest time from the currently active sampler config.
- Queue payloads are transient and may use compact binary storage; do not optimize them for ad hoc SQL readability at the expense of runtime throughput.
- Havana training and inference samplers must support nested discrete domains and preserve the full grid topology in persisted snapshots for restore/materialization.
- Havana training runs in deterministic lockstep windows: it may keep producing while earlier batches from the same window are still in flight, but must cap in-flight+ingested production to at most one `samples_for_update` window at a time, then pause until that window is fully ingested before updating the grid and continuing.
- Sample tasks must force an initial small batch round-trip before normal queue ramp-up so an accumulator snapshot is persisted immediately at task start, and must persist the accumulator again when the task completes.
- Sampler-aggregator completed-batch ingestion should advance from a persisted `batch.id` cursor, not rescan the whole run on every tick.
- Sampler-aggregator hot-loop control should reuse queue snapshots where possible and prefer direct evaluator counts over materializing full node rows.
- `sampler_aggregator_runner_params.frontend_sync_interval_ms` controls how often frontend-facing accumulator state is refreshed during sampling; full sampler resume checkpoints are persisted only on unassignment/pause, and task completion still forces a final accumulator flush.
- Evaluators use a fixed single-slot latent prefetch and single-slot async submit pipeline to hide DB latency. Materialization and evaluation remain strictly one batch at a time.
- Evaluators are stateless across reconcile-down. On stop they should drain already-claimed local latent batches without claiming new work, not persist evaluator state.
- Recoverable evaluator/materializer/batch-transform errors are batch-local: requeue the claimed batch with `last_error`/`retry_count` and keep the run alive. Evaluator role failures must not fail the run; sampler-aggregator role failures may fail the active task because sampler state owns task progress.
- Deleting a run is immediate control-plane teardown: clear desired/current node assignments for that run first, then delete the run and cascading data. Do not wait for pause/drain persistence.
- `node run` uses a tiny outer control-plane pool; role-specific PostgreSQL worker pool sizing lives on `evaluator_runner_params.db_pool_size` and `sampler_aggregator_runner_params.db_pool_size`.
- Sampler queue settings live under the nested TOML table `[sampler_aggregator_runner_params.queue]`.
- Sample tasks may optionally set `[task.queue_tuning]` to override queue knobs for that task only. Effective queue config is `run_spec.queue` overlaid by `task.queue_tuning`.
- Queue tuning updates are allowed for `pending` and `active` sample tasks via the explicit admin endpoint `POST /api/runs/:id/tasks/:task_id/queue-tuning`; non-sample tasks must reject queue-tuning updates.
- Live sampler runners must periodically refresh active-task queue tuning from storage and hot-apply it without restarting the role runner.
- `sampler_aggregator_runner_params.queue.queue_buffer` is the single public queue buffer control. The runner targets about `queue_buffer * active_evaluator_count` pending batches. `0.0` is the most aggressive setting and lets pending work drain to zero when the sampler cannot refill fast enough; larger values keep more pending work buffered. `max_queue_size` remains the hard cap.
- `sampler_aggregator_runner_params.queue.max_batches_per_tick` is a hard per-tick production cap and must apply to every sampler production path, including the forced initial round-trip batch.
- `sampler_aggregator_runner_params.queue.max_concurrent_insert_tasks` bounds how many sampler queue insert tasks may write batches concurrently on the shared process DB pool.
- Batch-size control belongs to the sampler queue (`sampler_aggregator_runner_params.queue.target_batch_eval_ms`, `batch_size_deadband_ratio`, `batch_size_cooldown_ticks`, `max_batch_size`, and queue-maintained `batch_size_current` tuning from completed-batch eval timings), not to sampler runner orchestration.
- Queue refill hysteresis belongs to the sampler queue (`pending_refill_low_ratio`, `pending_refill_high_ratio`) and should gate refill using low/high pending watermarks scaled by active evaluators and `queue_buffer`.
- The sampler queue owns the local pending-insert buffer, local processed buffer, and completed-batch cleanup scheduling. Normal ticks must poll these buffers and drain finished background tasks without blocking on database latency, while pause/unassign must drain the local queue fully before persisting the sampler checkpoint.
- Sampler frontend sync is lightweight and periodic: it updates `runs.current_observable` (current accumulator payload), appends `persisted_observable_snapshots`, and records performance snapshots. Full sampler resume state belongs in `run_sampler_checkpoints`, which is overwritten on unassignment/pause and contains the full sampler-aggregator checkpoint blob.

## Panels And Dashboard
- Backend visualization uses the generic panel model in `src/server/panels.rs`.
- The frontend should render panels generically; it should not reimplement domain projections or panel merge semantics.
- Table panel presentation and selection metadata must be backend-owned: use `visible_column_indices` and `row_keys` on table state instead of frontend panel-id-specific column hiding or row-key inference from visible cells.
- Run-level derived/exposed artifacts are persisted in `runs.exposed_info` as typed serde payloads; generate them lazily on first panel/API access, keyed by deterministic cache keys, and avoid manual ad-hoc JSON object construction.
- GammaLoop sample observables should project a histogram bundle table whose payload includes the histogram bins; the frontend renders the selected-histogram chart client-side as a stepped histogram with bin error bars and a linear/log y-scale toggle, and table row selection should drive the selected histogram using the live bundle rows.
- When a run-level complex target is configured, GammaLoop sample estimate history panels should include target overlays (real/imag target lines), and GammaLoop estimate summary should include target-comparison deltas (sigma and percent) for real/imag components.
- Panel APIs are poll-based: clients send an optional opaque `cursor`, plus `panel_state` and `panel_actions`; the backend returns `PanelResponse`.
- `append` is only valid when the backend can safely extend existing state; otherwise it must send `replace`.
- Panel specs may include simple width hints such as `compact`, `half`, and `full`.
- Sample task panels must receive a resolved effective accumulator kind from the stage/source resolver and must not probe persisted observable payloads to guess the accumulator type.
- Run info, task output, worker details, performance, and engine config should stay backend-owned.
- Tick breakdown bar panels must visualize only synchronous runner-tick work. Concurrent pipeline/queue work and wait/stall time must be surfaced in separate metrics panels, not stacked into the tick bar.
- Max-weight diagnostics should keep impact ranking based on weighted products, while exposing decomposed factor values (for example integrand and jacobians) when available.
- Dashboard auth is operator-oriented: read-only endpoints may stay open, while explicit steering endpoints require admin auth.
- The Runs tab `Copy Run TOML` action is a read-only export and should return run-add compatible TOML including run config plus successfully completed tasks as `[[task_queue]]`.
- Dashboard steering should use explicit endpoints such as `pause`, `assign`, `unassign`, `append task`, `remove pending task`, `create run`, `clone run`, and `remove run`, not generic patch endpoints.
- Dashboard auth is intended for small trusted deployments behind HTTPS.
- Run and task templates should be simple `.toml` files under shared template roots (for example `templates/runs` and `templates/tasks`) served from server-configured directories; the frontend should treat them as editable starting points, not as a second schema, and may persist/delete template files via explicit admin-protected template endpoints.
- Runtime database and tracing settings should come from `ops/local/config/runtime.toml` by default, with an optional global `--runtime-config <PATH>` override.
- Runtime resource lookup roots should live under `runtime.resources.roots` and be environment-owned (for example UBELIX workspace `states`), not inferred from binary location.
- Default config paths (`ops/local/config/runtime.toml`, `ops/local/config/server.toml`, `ops/local/config/deploy.toml`, and run-add defaults) must work even when those files are absent next to the binary; load built-in defaults from `src/config_defaults/*.toml` for fallback and never use `CARGO_MANIFEST_DIR` as a runtime fallback base.
- Local Postgres lifecycle commands should live under `gammaboard db ...` and use the shared runtime config instead of separate env-driven just recipes; `just db-reset` may wrap `gammaboard db stop`, `gammaboard db delete --yes`, and `gammaboard db start` for convenience.
- Local Postgres tuning lives under `runtime.local_postgres`; keep latency-sensitive queue defaults explicit there, including WAL/checkpoint settings and whether `synchronous_commit` is relaxed for local throughput.
- Local `gammaboard db start` should always enable `pg_stat_statements` preload and extension creation for the configured database.
- Server API bind, allowed origins, secure cookie policy, `allow_db_admin` policy, `allow_local_node_spawn` policy, dashboard auth secrets, and template directories should come from `ops/local/config/server.toml` by default, with an optional `gammaboard server --server-config <PATH>` override.
- Foreground deploy lifecycle should live under `gammaboard deploy run ...`, with `ops/<env>/config/deploy.toml` owning frontend HTTP exposure, static-site serving, and cleanup timing, while selecting which `ops/<env>/config/server.toml` backend profile to run.
- Deploy CLI overrides must be propagated consistently to supervised children. If `gammaboard deploy run` changes runtime/server values such as database URL, Postgres state paths, or API port, the backend child must receive matching CLI overrides.
- ITPhlies deploy supports multiple concurrent instances by selecting a frontend port in the just wrapper. The wrapper derives API/Postgres ports, DB name, and Postgres state dirs from that port, and deploy-generated nginx config/pid/temp paths must remain instance-scoped.
- Deploy HTTP config also owns nginx access-log visibility; interactive/local-style profiles should disable access logs unless explicitly needed for debugging.
- `gammaboard deploy run` must launch the backend with the same active `--runtime-config` path so server-managed node auto-run workers inherit the intended database and tracing settings.
- `gammaboard deploy run` shutdown is always coordinated through node shutdown, not run pause: request graceful shutdown for all live nodes, wait for active sampler-aggregator roles to persist state and clear, then stop backend/nginx and finally local Postgres.
- The dashboard control shutdown endpoint must use the same graceful node shutdown path before exiting the backend process; the deploy parent then performs final service/database cleanup.
- `gammaboard server` remains the direct foreground backend path; `gammaboard deploy` is orchestration around it, not a replacement for it.
- Server TOML should be explicit; do not rely on implicit defaults for required server settings.
- `gammaboard server` should terminate immediately on `Ctrl-C` (no graceful-drain wait path).

## Logging And Read APIs
- Runtime logs are persisted through the tracing pipeline into PostgreSQL.
- CLI console output should stay minimal and operator-focused; recurring runtime diagnostics should go to persisted runtime logs, not tracing `fmt` spam.
- Worker performance history is append-only, and dashboard "latest" performance reads should come from dedicated latest tables maintained on write, not recomputed latest-per-worker views over history.
- Persisted sampler runtime performance snapshots should include monotonic sampler-active uptime (pause intervals excluded) and total completed samples so frontend history axes can switch between wall time, sampler uptime, and completed-samples progress.
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
