# AGENTS

Use this file for architecture and implementation rules. Use `README.md` for setup and normal usage.

## Ownership
- `src/api/*`: typed use-cases shared by CLI/server; keep adapters thin.
- `src/core/*`: contracts, run/task types, traits, errors.
- `src/evaluation/*`, `src/sampling/*`, `src/runners/*`: evaluator/sampler semantics, queues, runtimes.
- `src/stores/*`: PostgreSQL queries/read models. `src/server/*`: API and panels. `src/cli/*`: parsing/bootstrap.

## Core Model
- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, and snapshots.
- Concrete evaluator batches are `Vec<Point>`. Point weights are named `weight_factors`; effective weight is `total_weight`.
- Run-global layout metadata uses `Domain`. Do not reintroduce `PointSpec` as run layout.
- `RunSpec` should keep immutable run-global state only. Task-varying sampler, materializer, batch-transform, and accumulator choices belong on tasks or stored integration defaults.
- Run names are human-facing and not unique; ambiguous CLI name references must fail and print matches.
- Runs are driven by persisted `run_tasks`; evaluator batches are lower-level queue items.
- `run add` without `task_queue` creates an idle run and immediately persists the root queue-empty `run_stage_snapshot`.
- Run lifecycle is derived from control-plane state. Do not add a persisted run status column unless explicitly requested.
- Pausing a run clears desired assignments so workers reconcile down cleanly.

## Nodes
- Node identity is `nodes.name` plus `nodes.uuid`: `name` is the operator handle, `uuid` is the live process incarnation.
- Nodes register/renew with one announce operation. Lease renewal must be independent from role ticks; announce failure for 30 seconds shuts the node down.
- Reconcile polling uses fast-start jittered backoff (`50ms`, `*2.0`, cap `2s`) and resets on meaningful role/task changes.
- Expired leases must not keep desired/current assignments alive; assignment writes and replacement announces should clear stale assignments opportunistically.
- Desired/current assignments live directly on `nodes`. At most one sampler-aggregator may be assigned to a run; many evaluators are allowed.
- `node run` must begin graceful shutdown immediately on `Ctrl-C` and `SIGTERM`, expire its lease promptly, and drain/reconcile according to its role.
- Control shutdown uses the shared graceful node shutdown API: clear desired assignments, request all nodes to stop, wait for active sampler-aggregator roles to persist/clear, then stop control-owned services.
- Node startup intent lives in `node_launch_requests`, not `nodes`. A request may represent many workers; resolver-specific details belong in JSON `args`/`result`.
- External launchers use the dashboard API, not direct SQL. Launch states are `pending`, `starting`, `running`, `failed`, `canceled`; `running` is reconciled from live leases.

## Tasks And Snapshots
- Task sequencing lives in `src/core/tasks.rs`. Use the single `RunTaskSpec` shape end-to-end.
- Operator-facing task identity is `name`; names are unique per run and optional in TOML with stable defaults.
- Snapshots are the branchable state timeline; tasks may produce snapshots but are not canonical branch identities.
- Every run has a root snapshot (`sequence_nr = 0`, `task_id = null`); there is no reserved `init` task.
- Task transitions must restore runtime state from persisted `run_stage_snapshots`, not in-memory handoff only.
- Cloning from a stage snapshot must not copy queued tasks; the clone starts idle at that snapshot.
- Source specs support omitted/`"latest"`, `{ from_name = "<task-name>" }`, or `{ config = ... }`; do not add legacy snapshot-id fallbacks.
- Task preflight belongs on task insertion; appended tasks validate against current or referenced stage snapshots before persistence.
- Deleting a run is immediate control-plane teardown: clear assignments for that run, then delete the run and cascading data. Do not wait for pause/drain persistence.

## Sampling And Evaluation
- `batch_transforms` is stage state: omitted inherits, `batch_transforms = []` clears inherited transforms.
- Sample tasks may omit `sampler_aggregator`/`accumulator` to reuse previous effective stage. First executable accumulator use must be established explicitly, usually with `set_accumulator`; there is no run-level accumulator default.
- Scalar and complex accumulator display projections, including named discrete histograms, belong to the effective accumulator config and are inherited with that accumulator state.
- Explicit sampler configs start fresh, except `havana_inference`, which resolves its configured handoff source.
- Sample stop conditions use `stop_condition = { max_samples, absolute_error, relative_error, projection = "real" | "imag" | "abs" }`.
- GammaLoop dimensions are inferred from the integrand. Do not configure `continuous_dims` or `discrete_dims` for `evaluator.kind = "gammaloop"`.
- GammaLoop evaluation uses x-space sampling. `post_load_commands` are allowed only as in-memory post-load changes.
- Relative evaluator resources resolve through ordered `runtime.resources.roots`; absolute paths are used as-is.
- Python evaluator/sampler protocols live in `python_api/python_workers/`; keep them compatible with `python_api/python_api/evaluator.py` and `python_api/python_api/sampler.py`.
- Python scalar evaluators/samplers use fixed rectangular homogeneous batches. Discrete shape is explicit via evaluator `discrete_cardinalities` or derived from the run domain for samplers; sampler `weights` become `sampler_weight`.
- Accumulator payloads persisted or exposed through APIs must be JSON-safe. Do not emit raw `NaN` or infinite floats.
- `accumulator = { config = "gammaloop" }` is GammaLoop-only and persists native histogram snapshots plus evaluation diagnostics.
- `image` and `plot_line` tasks declare their accumulator family explicitly and start with fresh full accumulators.
- `pdf_adaptation_*` tasks use sampler-owned persisted output payloads for frontend panels; `runs.current_observable` remains the task's accumulator payload.
- Claimed batches are fenced by live node ownership. Recoverable evaluator/materializer/transform errors are batch-local and should requeue with `last_error`/`retry_count`; sampler-aggregator failures may fail the active task.

## Queue And Performance
- Whether evaluator training values are required is a per-batch persisted contract (`batches.requires_training_values`).
- Queue payloads are transient and may use compact binary storage; do not optimize them for ad hoc SQL readability at runtime cost.
- Havana training uses deterministic lockstep windows capped by one `samples_for_update` window before grid update.
- Sample tasks force an initial small batch round-trip for an early accumulator snapshot, then persist again on completion.
- Sampler ingestion advances from a persisted `batch.id` cursor. Full resume state belongs in `run_sampler_checkpoints`.
- Evaluators are stateless across reconcile-down and drain already-claimed local latent batches without claiming new work.
- Queue knobs live under `[sampler_aggregator_runner_params.queue]`; sample task overrides live under `[task.queue_tuning]`.
- The sampler queue owns batch-size control, local buffers, refill hysteresis, insert concurrency, and pause/unassign draining before checkpoint persistence.

## Panels And Dashboard
- Backend visualization uses the generic panel model in `src/server/panels.rs`; frontend renders panels generically.
- Run info, task output, worker details, performance, engine config, table visibility, row keys, and domain projections are backend-owned.
- Panel APIs are poll-based and return `PanelResponse`; `append` is valid only for safe extension.
- Sample task panels receive a resolved effective accumulator kind when sample-specific panels render; do not probe persisted payloads to guess it.
- Run-level derived artifacts live in `runs.exposed_info` as typed serde payloads, generated lazily with deterministic cache keys.
- Dashboard steering uses explicit admin endpoints, not generic patch endpoints. `Copy Run TOML` is read-only and run-add compatible.
- Runtime logs persist through tracing into PostgreSQL. Keep CLI console output minimal and operator-focused. Read APIs serialize `BIGINT` ids as strings.

## Config And Deploy
- Runtime, server, and deploy config live under `ops/<env>/config/*.toml`; defaults come from `src/config_defaults/*.toml`. Do not use `CARGO_MANIFEST_DIR` as a runtime fallback.
- `gammaboard db ...` owns local Postgres lifecycle and uses active runtime config; `db start` enables `pg_stat_statements`.
- `gammaboard deploy run` supervises nginx/backend/Postgres, launches the backend with the same active runtime config and matching CLI overrides, and shuts down via graceful node shutdown.
- Direct `gammaboard server` exits immediately on `Ctrl-C`.
- Multi-instance deploys use a deploy `port_offset` that shifts configured frontend/API/Postgres ports and isolates Postgres state dirs/log paths per instance; keep nginx config/pid/temp paths instance-scoped by frontend port.

## Policy
- No backward-compat requirement by default. Prefer direct current-schema migrations unless compatibility is explicitly requested.
- Do not persist evaluator/sampler init metadata on runs or expose it in run panels/APIs unless explicitly requested.
- Fault-injection parameters on test evaluators/samplers must stay optional and default-off.
- If you change architecture, runtime behavior, CLI behavior, or config shape, update this file. If normal setup or operator workflow changes, update `README.md`.
## Required Checks
- `cargo fmt`, `cargo check -q`, `cargo test -q`, `just test-e2e`

- Prefer small coherent stages. After a stage is ready, provide a concrete commit message as a bare fenced `text` block with no label or commentary around it.
