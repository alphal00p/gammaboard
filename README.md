# Gammaboard

Gammaboard runs distributed numerical integration jobs using PostgreSQL as the shared runtime state.

At a high level:
- `gammaboard run` and `gammaboard node` commands manage run lifecycle and desired role assignments.
- `gammaboard run-node` polls desired assignment in PostgreSQL and starts/stops one local worker role loop (`evaluator` or `sampler_aggregator`) to match desired state.
  Role startup failures are capped (`--max-start-failures`, default `3`); once reached for one desired role target, `run-node` stops restarting that target until assignment changes.
- CLI console logging is tracing-based and target-split:
  `INFO` for `gammaboard*` targets and `WARN` for external crate targets by default.
  Use global `-q/--quiet` to suppress all `INFO` output and keep warnings/errors only.
- `gammaboard server` exposes run progress, aggregated results, and worker logs for the dashboard.
- Dashboard UI is split into `Runs` and `Workers` tabs:
  run-specific panels are shown only in `Runs`, while worker overview/details/logs are shown in `Workers`.
  `Runs` also includes a compact warn/error log panel with a shortcut to full logs in `Workers`.
- Aggregated-history polling uses sampled range reads (`GET /api/runs/:id/aggregated/range`)
  with explicit `latest` in the response.
- Runtime processes (`run-node`, `server`, and control commands) initialize tracing with a DB sink.
  Context-tagged events are persisted into `runtime_logs`; worker dashboard logs read
  `source='worker'`.
  The tracing layer writes through the store abstraction (`RuntimeLogStore`), with SQL implemented
  in the PostgreSQL store/query layer.
  Set `GAMMABOARD_DISABLE_DB_LOGS=1` to disable DB log persistence while keeping console tracing.
  DB sink thresholds are configurable via `GAMMABOARD_DB_LOG_LEVEL` (for `gammaboard*` targets)
  and `GAMMABOARD_DB_EXTERNAL_LOG_LEVEL` (for external targets), each in
  `off|error|warn|info|debug|trace`.
- Worker performance is persisted as history snapshots in role-split tables:
  `evaluator_performance_history` and `sampler_aggregator_performance_history`.
- `gammaboard run add` runs a typed preprocessing + deep preflight pipeline before DB insert;
  it derives `point_spec` from the evaluator config and validates engine compatibility by
  constructing engines from typed config enums ahead of persistence, including a one-point
  sampler -> parametrization -> evaluator dry-run.

## Quick Start

### Prerequisites
- Rust (edition 2024)
- PostgreSQL 16
- `sqlx` CLI (`cargo install sqlx-cli --no-default-features --features postgres`)
- Node.js + npm (only if you run the dashboard frontend)
- Docker (optional, for local Postgres via `docker-compose`)

### Fastest local run
1. Start everything and run a live backend test:
   - `just live-test-basic`
   - this provisions two runs: a `unit + naive_monte_carlo + scalar` run and
     a Symbolica 3D Gaussian run from `configs/symbolica-live-test.toml`.
2. Optional: run only the GammaLoop scenario:
   - `just live-test-gammaloop`
   - this provisions only `configs/gammaloop-triangle.toml`.
3. Optional: start backend + frontend dashboards:
   - terminal 1: `just serve-backend`
   - terminal 2: `just serve-frontend`
   - each `serve-*` command stops its own previous process before starting.

Useful stop commands:
- `just stop-workers`
- `just stop-serving`
- `just stop` (stops all runs via `gammaboard run stop -a`, then stops serving)
- `just restart-db`
- `just start-db` uses `docker-compose up -d --wait`, so migrations run only after
  PostgreSQL healthcheck reports ready.

## Manual Flow

1. Create a run from TOML config:
- `cargo run --bin gammaboard -- run add configs/live-test-unit-naive-scalar.toml`

2. Start nodes:
- `cargo run --bin gammaboard -- run-node --node-id node-a --poll-ms 1000`
- `cargo run --bin gammaboard -- run-node --node-id node-b --poll-ms 1000`
  - optional: tune restart cap with `--max-start-failures <N>`

Role selection is fully controlled by desired assignments in the DB.

3. Assign roles:
- `cargo run --bin gammaboard -- node assign node-a evaluator <RUN_ID>`
- `cargo run --bin gammaboard -- node assign node-b sampler-aggregator <RUN_ID>`

4. Start the run:
- `cargo run --bin gammaboard -- run start <RUN_ID> [<RUN_ID> ...]`
- `cargo run --bin gammaboard -- run start -a`

Pause/stop runs (also clears desired assignments so workers stop on next reconcile):
- `cargo run --bin gammaboard -- run pause <RUN_ID> [<RUN_ID> ...]`
- `cargo run --bin gammaboard -- run pause -a`
- `cargo run --bin gammaboard -- run stop <RUN_ID> [<RUN_ID> ...]`
- `cargo run --bin gammaboard -- run stop -a`

Stop run-node processes:
- `cargo run --bin gammaboard -- node stop <NODE_ID> [<NODE_ID> ...]`
- `cargo run --bin gammaboard -- node stop -a`

Remove runs:
- `cargo run --bin gammaboard -- run remove <RUN_ID> [<RUN_ID> ...]`
- `cargo run --bin gammaboard -- run remove -a`

## Configuration

Run configuration is provided as TOML.
- Backend serve port is configured via environment variable
  `GAMMABOARD_BACKEND_PORT` (for `just serve*` and `gammaboard server` when `--bind` is omitted).
- Frontend API base URL is configured via `REACT_APP_API_BASE_URL`.
- Backend DB pool initialization retries transient connection failures with backoff
  (helps when Postgres is still coming up).
- `gammaboard run add <file.toml>` first loads `configs/default.toml`, then deep-merges the run file on top (run file values win).
- Runtime defaults for run/integration payloads are not sourced from Rust struct defaults; keep `configs/default.toml` complete.
- Run display name is configured via top-level `name` and stored in `runs.name`.
- Optional top-level `target` is stored as opaque JSON in `runs.target` (backend pass-through).
- Run lifecycle status is persisted in `runs.status` and controlled by
  `gammaboard run` commands (`start`, `pause`, `stop`).
- Engine and runner params are stored in `runs.integration_params` using component sections
  (`evaluator`, `sampler_aggregator`, `observable`, `parametrization`) with `kind` tags.
- Point dimensions are stored in `runs.point_spec`.
- Batches are stored in `batches.points` as compact flat arrays (`continuous`, `discrete`) plus per-sample `weights`, with explicit 2D shape metadata.
- Evaluators return one `BatchResult` per batch with aggregated `observable` JSON and optional `values: Option<Vec<f64>>` training signal.
- Each enqueued batch carries `batches.requires_training`; evaluator results may omit training values for batches where this flag is `false`.
- Sampler training completion is persisted once per run in `runs.training_completed_at`
  when the sampler reports training inactive.
- Runtime engines are constructed via typed config enums (`EvaluatorConfig`, `SamplerAggregatorConfig`, `ParametrizationConfig`, `ObservableConfig`) using `build()` methods that return boxed trait objects.
- Evaluator implementations receive an `ObservableConfig` during `eval_batch` and build batch-local observable state from it.
- Evaluator and sampler-aggregator implementations may expose initialization metadata via `get_init_metadata`; `run add` preprocessing resolves and persists these payloads into `runs.evaluator_init_metadata` and `runs.sampler_aggregator_init_metadata` at run creation.
- `gammaloop` evaluator parameters use:
  `state_folder`, optional `model_file`, optional `process_id`, optional `integrand_name`,
  optional `momentum_space`, optional `use_f128`, and optional `training_projection`
  (`real|imag|abs|abs_sq`); it evaluates points with the same per-point flow as
  GammaLoop Python `batched_inspect`.
- Local GammaLoop compatibility patch (in sibling repo `../gammaloop`):
  - `src/initialisation.rs`: replaced eager access to `INBUILTS.conj` with
    `spenso_conj_symbol()`.
  - `src/utils.rs`: replaced `INBUILTS.conj` keys in `FUN_LIB`, `PARAM_FUN_LIB`,
    and `INT_FUN_LIB` with `spenso_conj_symbol()`, and added helper
    `spenso_conj_symbol()` using `symbolica::try_symbol!`.
  - Purpose: avoid panic from `symbol!` redefinition path
    (`"Symbol spenso::conj redefined with new attributes"`), which otherwise
    poisons init and causes repeated worker restarts.
- `symbolica` evaluator parameters use `expr` and `args` (argument symbols).
- `symbolica` evaluator build artifacts are written to a per-engine temporary directory under `./.evaluators` and cleaned up when the evaluator instance is dropped.
- `unit` evaluator parameters are `{}` and always return per-sample value `1.0`.
- Observable ingestion in evaluators is capability-based (`as_scalar_ingest` / `as_complex_ingest`) instead of matching concrete observable enum variants.
- `complex` observable accepts both complex samples and scalar samples (scalar values are cast to `real + 0i`).
- `unit_ball` parametrization maps unit-hypercube continuous samples to the unit ball and scales per-sample weights by the corresponding Jacobian.
- `spherical` parametrization maps `[0,1)^3` to `R^3` with:
  - radial map `r = u_r / (1 - u_r)`,
  - full spherical direction map with `cos(theta) = 2*u_theta - 1` and `phi = 2*pi*u_phi`,
  - Jacobian factor `4*pi * r^2 / (1 - u_r)^2`.
- Sampler-aggregator engines produce one batch per call; the sampler-aggregator runner controls how many batches are produced each tick (`max_batches_per_tick`) and enforces pending-queue limits.
- Sampler-aggregator runner adapts produced batch size toward
  `target_batch_eval_ms`, bounded by `max_batch_size`.
- Sampler-aggregator runner uses `max_queue_size` as the queue throttle.
- Sampler-aggregator runner queue throughput is tuned by
  `target_queue_remaining` (`0 <= value <= 1`): from observed evaluator
  drain-per-tick, the runner targets a pending depth that leaves roughly this
  fraction in queue by the next tick. `1.0` disables lean-throttling and fills
  queue up to hard limits (`max_queue_size`, `max_batches_per_tick`).
- Sampler-aggregator performance snapshots are persisted periodically via
  `sampler_aggregator_runner_params.performance_snapshot_interval_ms`.
- Sampler-aggregator engines can attach optional process-local batch context to produced batches; this context is passed back during training ingestion and is not persisted in PostgreSQL.
- Sampler-aggregator engines may optionally throttle per-tick production via `SamplerAggregator::get_max_samples` (default `None` = no engine-specific sample cap); under a cap, the runner builds a near-uniform per-tick batch plan that hits the sample target exactly.
- Runner params in the integration payload are strongly typed (`EvaluatorRunnerParams`, `SamplerAggregatorRunnerParams`).
- `configs/live-test*.toml` intentionally sets all known fields (including default-valued ones) as reference templates.
- Observable snapshots are serde-derived state payloads. For `scalar`, the snapshot fields are:
  `count`, `sum_weighted_value`, `sum_abs`, `sum_sq`.

Example: `configs/live-test-unit-naive-scalar.toml`

```toml
name = "live-test"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[sampler_aggregator]
kind = "naive_monte_carlo"

[observable]
kind = "scalar"

[parametrization]
kind = "none"

[evaluator_runner_params]
min_loop_time_ms = 5
performance_snapshot_interval_ms = 2000

[sampler_aggregator_runner_params]
min_poll_time_ms = 100
performance_snapshot_interval_ms = 2000
target_batch_eval_ms = 400.0
target_queue_remaining = 0.5
lease_ttl_ms = 5000
max_batch_size = 64
max_batches_per_tick = 8
max_queue_size = 128
completed_batch_fetch_limit = 1024
```

Havana-specific sampler params:
- `batch_size` is not a Havana parameter; produced sample count is controlled by
  the adaptive sampler runner
  (`max_batch_size`, `target_batch_eval_ms`, `target_queue_remaining`).
- Point dimensions are derived from evaluator config (`Evaluator::get_point_spec`) and persisted
  as `runs.point_spec`. Sampler params must not duplicate dimension fields.
- Evaluators that cannot infer dimensions from intrinsic model structure (for example `unit`)
  must include `continuous_dims`/`discrete_dims` in `[evaluator]`.
- `samples_for_update`: number of ingested training samples between grid updates.
- `initial_training_rate`: training rate at sample 0.
- `final_training_rate`: training rate at the end of training.
- `stop_training_after_n_samples`: required cap on total trained samples; once reached, Havana keeps producing batches but skips training updates.
  When this is set, Havana uses exponential interpolation from
  `initial_training_rate` to `final_training_rate` using absolute
  `samples_ingested`.

`observable.kind` and `parametrization.kind` are configured independently from
evaluator/sampler kinds, so runs can mix-and-match compatible engines.
Compatibility is validated at runner startup before evaluator work begins.

### Frontend Observable Notes
- The dashboard computes scalar mean as `sum_weighted_value / count` from aggregated snapshots.
- Non-scalar observables are shown using implementation-specific rendering and JSON fallback.

## Current Status

- Test-only engine implementations are currently wired by default.
- Runs can be reassigned at runtime by updating desired assignments via `gammaboard node`.
- Sampler-aggregator state is in-memory only; completed batches are consumed and deleted after ingestion.
- Each `gammaboard run-node` process executes at most one active role task at a time, with role selection driven by DB desired assignments.
- `gammaboard node stop` requests node shutdown via DB; each `gammaboard run-node`
  consumes and clears a one-shot shutdown signal before exiting.
- Role lifecycle, lease, heartbeat, and tick failure events from `gammaboard run-node` are persisted
  in `runtime_logs` under `source='worker'` (indexed by source+run/worker, with `node_id`
  populated from worker tracing context when available).
- Runtime log sink filtering uses tracing event targets:
  `gammaboard*` targets and external targets have separate level thresholds.
- Worker logs are readable via `GET /api/runs/:id/logs` and shown in the dashboard's
  **Logs** tab. Each entry includes `id`, `ts`, `level`, `message`, and
  structured `fields` (no dedicated `event_type` column).
- Aggregated-history chart data is read via:
  - `GET /api/runs/:id/aggregated/range?start=<i64>&stop=<i64>&max_points=<i64>&last_id=<optional>`
  - negative `start`/`stop` are relative to newest id (`-1` = newest)
  - backend derives sampling `step` automatically from resolved absolute range and `max_points` (step is power-of-two rounded for stable coarsening)
  - if `last_id` no longer matches the current sampling grid (e.g. step changed), response sets `reset_required=true` and frontend should replace buffered history instead of appending
  - response always includes an explicit `latest` snapshot field
- API responses serialize `BIGINT` identifiers (`id`, `latest_id`) as strings for
  JavaScript precision safety.
- Registered workers are readable via `GET /api/workers` (optional `run_id` query
  filter) and include per-run performance stats:
  evaluator `evaluator_metrics` (generic counters/timing),
  sampler `sampler_metrics` (generic counters/timing),
  sampler `sampler_runtime_metrics` (runner tuning/runtime metrics),
  plus optional engine diagnostics.
  Evaluator diagnostics are exposed as `evaluator_engine_diagnostics`.
  Sampler diagnostics are split into `sampler_runtime_metrics` (generic runner metrics)
  and `sampler_engine_diagnostics` (implementation-specific diagnostics).
- Performance history is available via:
  `GET /api/runs/:id/performance/evaluator` and
  `GET /api/runs/:id/performance/sampler-aggregator`
  (optional `limit`, `worker_id`). History rows are snapshot records ordered by
  `created_at` (no `window_start`/`window_end` fields).
- Worker-scoped performance history is available via:
  `GET /api/workers/:id/performance/evaluator` and
  `GET /api/workers/:id/performance/sampler-aggregator`
  (optional `limit`). Responses include the currently assigned `run_id`
  alongside snapshot entries.
- Worker-scoped performance history is available via:
  `GET /api/workers/:id/performance/evaluator` and
  `GET /api/workers/:id/performance/sampler-aggregator`.

## For Contributors

Engineering structure and maintenance rules live in `AGENTS.md`.
Batch domain types now live in `src/core/batch.rs` and are re-exported at crate root for compatibility.
Engine contracts/config-builders are split by role in `src/engines/{evaluator,sampler_aggregator,observable}/`.
Engine implementations should use `engines::BuildFromJson` for parameter decoding/validation to keep factory behavior consistent.
Evaluator and sampler engines expose `get_diagnostics()`; default output is `json!("{}")`.
Evaluator snapshots persist into `evaluator_performance_history.engine_diagnostics`
with generic counters/timing in `evaluator_performance_history.metrics`.
Sampler snapshots persist engine diagnostics in
`sampler_aggregator_performance_history.engine_diagnostics`,
generic counters/timing in `sampler_aggregator_performance_history.metrics`,
and runner tuning/runtime metrics in `runtime_metrics`.
Implementation enums use `strum` derives (`AsRefStr`, `Display`) and should be converted with `.as_ref()` when a string slice is required.
`IntegrationParams` and `RunSpec` now live in `src/engines/shared.rs`.
If you change architecture, CLI/config, or runtime behavior, update both `AGENTS.md` and this README in the same change.
