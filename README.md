# Gammaboard

Gammaboard runs distributed numerical integration jobs using PostgreSQL as the shared runtime state.

At a high level:
- `control_plane` decides which node should do which role for a run.
- `run_node` on each node starts/stops local worker loops to match that desired state.
- `server` exposes run progress and aggregated results for the dashboard.

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
2. Optional: include the frontend too:
   - `just live-test-with-frontend`

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

3. Assign roles:
- `cargo run --bin control_plane -- assign --node-id node-a --role evaluator --run-id <RUN_ID>`
- `cargo run --bin control_plane -- assign --node-id node-b --role sampler-aggregator --run-id <RUN_ID>`

4. Start the run:
- `cargo run --bin control_plane -- run-start --run-id <RUN_ID>`

## Configuration

Run configuration is provided as TOML.
- Engine and runner params are stored in `runs.integration_params`.
- Point dimensions are stored in `runs.point_spec`.
- Batches are stored in `batches.points` as compact flat arrays (`continuous`, `discrete`) with explicit 2D shape metadata.
- Evaluators return one `BatchResult` per batch: `values: Vec<f64>` (sampler training signal) and one aggregated `observable` JSON payload.

Example: `configs/live-test.toml`

```toml
evaluator_implementation = "test_only_sin"
sampler_aggregator_implementation = "test_only_training"

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

`observable_params` are consumed by the observable that is selected by
`evaluator_implementation` (not configured independently).

## Current Status

- Test-only engine implementations are currently wired by default.
- Runs can be reassigned at runtime by updating desired assignments via `control_plane`.
- Sampler-aggregator state is in-memory only; completed batches are consumed and deleted after ingestion.

## For Contributors

Engineering structure and maintenance rules live in `AGENTS.md`.
If you change architecture, CLI/config, or runtime behavior, update both `AGENTS.md` and this README in the same change.
