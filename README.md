# Gammaboard

Gammaboard runs distributed numerical integration jobs using PostgreSQL as the shared runtime state.

At a high level:
- `control_plane` decides which node should do which role for a run.
- `run_node` polls desired assignment in PostgreSQL and starts/stops one local worker role loop (`evaluator` or `sampler_aggregator`) to match desired state.
- `server` exposes run progress, aggregated results, and worker logs for the dashboard.
- `server` run stats SSE (`GET /api/runs/:id/stream`) uses one shared per-run polling
  loop with broadcast fanout to all connected clients.
- `run_node` emits structured worker logs via `tracing`; `target="worker_log"` events
  are persisted into `worker_logs`.
- Worker performance is persisted as history snapshots in role-split tables:
  `evaluator_performance_history` and `sampler_aggregator_performance_history`.

## Quick Start

### Prerequisites
- Rust (edition 2024)
- PostgreSQL 16
- `sqlx` CLI (`cargo install sqlx-cli --no-default-features --features postgres`)
- Node.js + npm (only if you run the dashboard frontend)
- Docker (optional, for local Postgres via `docker-compose`)

### Fastest local run
1. Start everything and run a live backend test:
   - `just live-test`
   - this provisions two runs: a basic `unit + naive_monte_carlo + scalar` run,
     and a Symbolica polynomial run (`x^2 + y^4`) with 2 evaluator workers.
2. Optional: start backend + frontend dashboards:
   - `just serve`

Useful stop commands:
- `just stop-workers`
- `just stop-serving`
- `just stop` (stops all runs via `control_plane run-stop -a`, then stops serving)
- `just restart-db`

## Manual Flow

1. Create a run from TOML config:
- `cargo run --bin control_plane -- run-add configs/live-test-unit-naive-scalar.toml`

2. Start nodes:
- `cargo run --bin run_node -- --node-id node-a --poll-ms 1000`
- `cargo run --bin run_node -- --node-id node-b --poll-ms 1000`

Role selection is fully controlled by desired assignments in the DB.

3. Assign roles:
- `cargo run --bin control_plane -- assign node-a evaluator <RUN_ID>`
- `cargo run --bin control_plane -- assign node-b sampler-aggregator <RUN_ID>`

4. Start the run:
- `cargo run --bin control_plane -- run-start <RUN_ID> [<RUN_ID> ...]`
- `cargo run --bin control_plane -- run-start -a`

Pause/stop runs (also clears desired assignments so workers stop on next reconcile):
- `cargo run --bin control_plane -- run-pause <RUN_ID> [<RUN_ID> ...]`
- `cargo run --bin control_plane -- run-pause -a`
- `cargo run --bin control_plane -- run-stop <RUN_ID> [<RUN_ID> ...]`
- `cargo run --bin control_plane -- run-stop -a`

Stop run-node processes from the control plane:
- `cargo run --bin control_plane -- node-stop <NODE_ID> [<NODE_ID> ...]`
- `cargo run --bin control_plane -- node-stop -a`

Remove runs:
- `cargo run --bin control_plane -- run-remove <RUN_ID> [<RUN_ID> ...]`
- `cargo run --bin control_plane -- run-remove -a`

## Configuration

Run configuration is provided as TOML.
- `control_plane run-add <file.toml>` first loads `configs/default.toml`, then deep-merges the run file on top (run file values win).
- Runtime defaults for run/integration payloads are not sourced from Rust struct defaults; keep `configs/default.toml` complete.
- Run display name is configured via top-level `name` and stored in `runs.name`.
- Run lifecycle status is persisted in `runs.status` and controlled by control-plane
  run commands (`run-start`, `run-pause`, `run-stop`).
- Engine and runner params are stored in `runs.integration_params`.
- Observable implementation is stored in `runs.observable_implementation`.
- Parametrization implementation and params are stored in `runs.integration_params`
  (`parametrization_implementation`, `parametrization_params`).
- Point dimensions are stored in `runs.point_spec`.
- Batches are stored in `batches.points` as compact flat arrays (`continuous`, `discrete`) plus per-sample `weights`, with explicit 2D shape metadata.
- Evaluators return one `BatchResult` per batch with aggregated `observable` JSON and optional `values: Option<Vec<f64>>` training signal.
- Each enqueued batch carries `batches.requires_training`; evaluator results may omit training values for batches where this flag is `false`.
- Sampler training completion is persisted once per run in `runs.training_completed_at`
  when the sampler reports training inactive.
- Runtime engines are constructed via factories (`EvaluatorFactory`, `SamplerAggregatorFactory`, `ParametrizationFactory`, `ObservableFactory`) that return boxed trait objects; implementation enums remain config-only.
- Evaluator implementations receive an `ObservableFactory` during `eval_batch` and build batch-local observable state from it.
- Evaluator and sampler-aggregator implementations may expose one-time initialization metadata via trait hooks (`get_init_metadata`); workers persist these payloads into `runs.evaluator_init_metadata` and `runs.sampler_aggregator_init_metadata` with write-once semantics (`NULL -> JSONB`).
- `symbolica` evaluator parameters use `expr` and `args` (argument symbols).
- `symbolica` evaluator build artifacts are written to a per-engine temporary directory under `./.evaluators` and cleaned up when the evaluator instance is dropped.
- `unit` evaluator parameters are `{}` and always return per-sample value `1.0`.
- Observable ingestion in evaluators is capability-based (`as_scalar_ingest` / `as_complex_ingest`) instead of matching concrete observable enum variants.
- `complex` observable accepts both complex samples and scalar samples (scalar values are cast to `real + 0i`).
- `spherical` parametrization maps unit-hypercube continuous samples to the unit ball and scales per-sample weights by the spherical-coordinate Jacobian.
- Sampler-aggregator engines produce one batch per call; the sampler-aggregator runner controls how many batches are produced each tick (`max_batches_per_tick`) and enforces pending-queue limits.
- Sampler-aggregator runner adapts produced batch size toward
  `target_batch_eval_ms`, bounded by `max_batch_size`.
- Sampler-aggregator runner uses `max_queue_size` as the queue throttle.
- Sampler-aggregator performance snapshots are persisted periodically via
  `sampler_aggregator_runner_params.performance_snapshot_interval_ms`.
- Sampler-aggregator engines can attach optional process-local batch context to produced batches; this context is passed back during training ingestion and is not persisted in PostgreSQL.
- Sampler-aggregator engines may optionally throttle per-tick production via `SamplerAggregator::get_max_samples` (default `None` = no engine-specific sample cap); under a cap, the runner builds a near-uniform per-tick batch plan that hits the sample target exactly.
- Runner params in the integration payload are strongly typed (`EvaluatorRunnerParams`, `SamplerAggregatorRunnerParams`).
- `configs/live-test*.toml` intentionally sets all known fields (including default-valued ones) as reference templates.
- Observable snapshots are serde-derived state payloads. For `scalar`, the snapshot fields are:
  `count`, `sum_weight`, `sum_abs`, `sum_sq`.

Example: `configs/live-test-unit-naive-scalar.toml`

```toml
name = "live-test"
evaluator_implementation = "unit"
sampler_aggregator_implementation = "naive_monte_carlo"
observable_implementation = "scalar"
parametrization_implementation = "none"

[point_spec]
continuous_dims = 1
discrete_dims = 0

[evaluator_runner_params]
min_loop_time_ms = 5
performance_snapshot_interval_ms = 2000

[evaluator_params]

[sampler_aggregator_runner_params]
min_poll_time_ms = 100
performance_snapshot_interval_ms = 2000
target_batch_eval_ms = 400.0
lease_ttl_ms = 5000
max_batch_size = 64
max_batches_per_tick = 8
max_queue_size = 128
completed_batch_fetch_limit = 1024

[sampler_aggregator_params]
continuous_dims = 1
discrete_dims = 0

[observable_params]

[parametrization_params]
```

Havana-specific sampler params:
- `batch_size` is not a Havana parameter; produced sample count is controlled by
  the adaptive sampler runner (`max_batch_size`, `target_batch_eval_ms`).
- `samples_for_update`: number of ingested training samples between grid updates.
- `initial_training_rate`: training rate at sample 0.
- `final_training_rate`: training rate at the end of training.
- `stop_training_after_n_samples`: required cap on total trained samples; once reached, Havana keeps producing batches but skips training updates.
  When this is set, Havana uses exponential interpolation from
  `initial_training_rate` to `final_training_rate` using absolute
  `samples_ingested`.

`observable_implementation` and `observable_params` are configured independently
from evaluator/sampler implementations, so runs can mix-and-match compatible
engines.
`parametrization_implementation` and `parametrization_params` are optional and
default to `"none"` and `{}` when omitted.
On write, `observable_implementation` is persisted in `runs.observable_implementation`
instead of the JSON blob.
Compatibility is validated at runner startup before evaluator work begins.

### Frontend Observable Notes
- The dashboard computes scalar mean as `sum_weight / count` from aggregated snapshots.
- Non-scalar observables are shown using implementation-specific rendering and JSON fallback.

## Current Status

- Test-only engine implementations are currently wired by default.
- Runs can be reassigned at runtime by updating desired assignments via `control_plane`.
- Sampler-aggregator state is in-memory only; completed batches are consumed and deleted after ingestion.
- Each `run_node` process executes at most one active role task at a time, with role selection driven by DB desired assignments.
- `control_plane node-stop` requests node shutdown via DB; each `run_node` consumes and clears a one-shot shutdown signal before exiting.
- Role lifecycle, lease, heartbeat, and tick failure events from `run_node` are persisted
  in `worker_logs` (indexed by `run_id` and `worker_id`).
- Worker logs are readable via `GET /api/runs/:id/logs` and shown in the dashboard's
  **Worker Logs** panel.
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

## For Contributors

Engineering structure and maintenance rules live in `AGENTS.md`.
Batch domain types now live in `src/core/batch.rs` and are re-exported at crate root for compatibility.
Engine contracts/factories are split by role in `src/engines/{evaluator,sampler_aggregator,observable}/`.
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
