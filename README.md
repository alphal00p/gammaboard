# Gammaboard

Gammaboard runs distributed numerical integration jobs using PostgreSQL as the shared runtime state.

At a high level:
- `control_plane` decides which node should do which role for a run.
- `run_node` polls desired assignment in PostgreSQL and starts/stops one local worker role loop (`evaluator` or `sampler_aggregator`) to match desired state.
- `server` exposes run progress, aggregated results, and worker logs for the dashboard.
- `run_node` emits structured worker logs via `tracing`; `target="worker_log"` events
  are persisted into `worker_logs`.

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
- Engine and runner params are stored in `runs.integration_params`.
- Observable implementation is stored in `runs.observable_implementation`.
- Point dimensions are stored in `runs.point_spec`.
- Batches are stored in `batches.points` as compact flat arrays (`continuous`, `discrete`) with explicit 2D shape metadata.
- Evaluators return one `BatchResult` per batch: `values: Vec<f64>` (sampler training signal) and one aggregated `observable` JSON payload.

Example: `configs/live-test.toml`

```toml
evaluator_implementation = "test_only_sin"
sampler_aggregator_implementation = "test_only_training"
observable_implementation = "scalar"

[point_spec]
continuous_dims = 1
discrete_dims = 0

[evaluator_runner_params]
min_loop_time_ms = 200

[evaluator_params]
min_eval_time_per_sample_ms = 2

[sampler_aggregator_runner_params]
interval_ms = 500
lease_ttl_ms = 5000
max_batches_per_tick = 1
max_pending_batches = 128
completed_batch_fetch_limit = 512

[sampler_aggregator_params]
batch_size = 64
training_target_samples = 2000
training_delay_per_sample_ms = 2

[observable_params]
```

`observable_implementation` and `observable_params` are configured independently
from evaluator/sampler implementations, so runs can mix-and-match compatible
engines.
On write, `observable_implementation` is persisted in `runs.observable_implementation`
instead of the JSON blob.
Compatibility is validated at runner startup before evaluator work begins.

## Current Status

- Test-only engine implementations are currently wired by default.
- Runs can be reassigned at runtime by updating desired assignments via `control_plane`.
- Sampler-aggregator state is in-memory only; completed batches are consumed and deleted after ingestion.
- Each `run_node` process executes at most one active role task at a time, with role selection driven by DB desired assignments.
- Role lifecycle, lease, heartbeat, and tick failure events from `run_node` are persisted
  in `worker_logs` (indexed by `run_id` and `worker_id`).
- Worker logs are readable via `GET /api/runs/:id/logs` and shown in the dashboard's
  **Worker Logs** panel.

## For Contributors

Engineering structure and maintenance rules live in `AGENTS.md`.
Batch domain types now live in `src/core/batch.rs` and are re-exported at crate root for compatibility.
Engine contracts/factories are split by role in `src/engines/{evaluator,sampler_aggregator,observable}/`.
Engine implementations should use `engines::BuildFromJson` for parameter decoding/validation to keep factory behavior consistent.
`IntegrationParams` and `RunSpec` now live in `src/engines/shared.rs`.
If you change architecture, CLI/config, or runtime behavior, update both `AGENTS.md` and this README in the same change.
