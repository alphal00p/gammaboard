# Gammaboard

Gammaboard runs distributed numerical integration jobs using PostgreSQL as the shared runtime state.

At a high level:
- `control_plane` decides which node should do which role for a run.
- `run_node` polls desired assignment in PostgreSQL and starts/stops one local worker role loop (`evaluator` or `sampler_aggregator`) to match desired state.
- `server` exposes run progress, aggregated results, and worker logs for the dashboard.
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
   - this provisions four semantically named runs with mixed evaluator/sampler/observable combinations, including two intentionally incompatible runs for error-surface testing.
2. Optional: start backend + frontend dashboards:
   - `just serve`

Useful stop commands:
- `just stop-workers`
- `just stop-serving`
- `just restart-db`

## Manual Flow

1. Create a run from TOML config:
- `cargo run --bin control_plane -- run-add --status pending --integration-params-file configs/live-test.toml`

2. Start nodes:
- `cargo run --bin run_node -- --node-id node-a --poll-ms 1000`
- `cargo run --bin run_node -- --node-id node-b --poll-ms 1000`

Role selection is fully controlled by desired assignments in the DB.

3. Assign roles:
- `cargo run --bin control_plane -- assign --node-id node-a --role evaluator --run-id <RUN_ID>`
- `cargo run --bin control_plane -- assign --node-id node-b --role sampler-aggregator --run-id <RUN_ID>`

4. Start the run:
- `cargo run --bin control_plane -- run-start --run-id <RUN_ID>`

## Configuration

Run configuration is provided as TOML.
- Run display name is configured via top-level `name` and stored in `runs.name`.
- Engine and runner params are stored in `runs.integration_params`.
- Observable implementation is stored in `runs.observable_implementation`.
- Point dimensions are stored in `runs.point_spec`.
- Batches are stored in `batches.points` as compact flat arrays (`continuous`, `discrete`) plus per-sample `weights`, with explicit 2D shape metadata.
- Evaluators return one `BatchResult` per batch: `values: Vec<f64>` (sampler training signal) and one aggregated `observable` JSON payload.
- Evaluator implementations receive an `ObservableFactory` during `eval_batch` and build batch-local observable state from it.
- Sampler-aggregator engines produce one batch per call; the sampler-aggregator runner controls how many batches are produced each tick (`max_batches_per_tick`) and enforces pending-queue limits.
- Sampler-aggregator runner passes explicit `nr_samples` from `sampler_aggregator_runner_params` into `produce_batch`.
- Sampler-aggregator engines can attach optional process-local batch context to produced batches; this context is passed back during training ingestion and is not persisted in PostgreSQL.
- Sampler-aggregator engines may optionally throttle per-tick production via `SamplerAggregator::get_max_batches` (default `None` = no engine-specific cap). Havana uses this for deterministic cycle limits and optional training stop.
- Runner params in the integration payload are strongly typed (`EvaluatorRunnerParams`, `SamplerAggregatorRunnerParams`) and defaulted server-side when omitted.
- `configs/live-test*.toml` intentionally sets all known fields (including default-valued ones) as reference templates.
- Observable snapshots are serde-derived state payloads. For `scalar`, the snapshot fields are:
  `count`, `sum_weight`, `sum_abs`, `sum_sq`.

Example: `configs/live-test.toml`

```toml
name = "live-test"
evaluator_implementation = "test_only_sin"
sampler_aggregator_implementation = "test_only_training"
observable_implementation = "scalar"

[point_spec]
continuous_dims = 1
discrete_dims = 0

[evaluator_runner_params]
min_loop_time_ms = 200
performance_snapshot_interval_ms = 5000

[evaluator_params]
min_eval_time_per_sample_ms = 2

[sampler_aggregator_runner_params]
interval_ms = 500
lease_ttl_ms = 5000
nr_samples = 64
performance_snapshot_interval_ms = 5000
max_batches_per_tick = 128
max_pending_batches = 128
completed_batch_fetch_limit = 512

[sampler_aggregator_params]
continuous_dims = 1
discrete_dims = 0
training_target_samples = 2000
training_delay_per_sample_ms = 2

[observable_params]
```

Havana-specific sampler params:
- `batches_for_update`: number of produced/ingested batches before one grid update cycle.
- `initial_training_rate`: training rate at batch 0.
- `final_training_rate`: training rate at the end of training.
- `stop_training_after_n_batches` (optional): hard cap on total produced training batches; once reached, sampler production throttles to zero.
  When this is set, Havana uses exponential interpolation from
  `initial_training_rate` to `final_training_rate` using absolute
  `batches_produced`.

`observable_implementation` and `observable_params` are configured independently
from evaluator/sampler implementations, so runs can mix-and-match compatible
engines.
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
- Role lifecycle, lease, heartbeat, and tick failure events from `run_node` are persisted
  in `worker_logs` (indexed by `run_id` and `worker_id`).
- Worker logs are readable via `GET /api/runs/:id/logs` and shown in the dashboard's
  **Worker Logs** panel.
- Registered workers are readable via `GET /api/workers` (optional `run_id` query
  filter) and include per-run performance stats:
  evaluator `avg_time_per_sample_ms`/`std_time_per_sample_ms`,
  sampler `avg_produce_time_per_sample_ms`/`std_produce_time_per_sample_ms`,
  sampler `avg_ingest_time_per_sample_ms`/`std_ingest_time_per_sample_ms`,
  plus evaluator/sampler batch+sample counters and diagnostics JSON emitted by
  engine `get_diagnostics()` hooks.
- Performance history is available via:
  `GET /api/runs/:id/performance/evaluator` and
  `GET /api/runs/:id/performance/sampler-aggregator`
  (optional `limit`, `worker_id`).

## For Contributors

Engineering structure and maintenance rules live in `AGENTS.md`.
Batch domain types now live in `src/core/batch.rs` and are re-exported at crate root for compatibility.
Engine contracts/factories are split by role in `src/engines/{evaluator,sampler_aggregator,observable}/`.
Engine implementations should use `engines::BuildFromJson` for parameter decoding/validation to keep factory behavior consistent.
Evaluator and sampler engines expose `get_diagnostics()`; default output is `json!("{}")`, and runner snapshots persist this into performance history rows.
Implementation enums use `strum` derives (`AsRefStr`, `Display`) and should be converted with `.as_ref()` when a string slice is required.
`IntegrationParams` and `RunSpec` now live in `src/engines/shared.rs`.
If you change architecture, CLI/config, or runtime behavior, update both `AGENTS.md` and this README in the same change.
