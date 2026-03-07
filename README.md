# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane, queue, and telemetry store.

## What It Does
- `gammaboard run` manages run lifecycle.
- `gammaboard node` manages desired worker assignments.
- `gammaboard run-node` reconciles one local process into one active role loop at a time: `evaluator` or `sampler_aggregator`.
- `gammaboard server` serves the dashboard read API.
- The dashboard exposes `Runs`, `Workers`, and `Logs` tabs.

## Quick Start

### Prerequisites
- Rust
- PostgreSQL 16
- `sqlx` CLI: `cargo install sqlx-cli --no-default-features --features postgres`
- Node.js + npm for the dashboard
- Docker optional, for local Postgres via `docker-compose`

### Fast local setup
1. Start the database:
   - `just start-db`
2. Start the backend:
   - `just serve-backend`
3. Start the frontend:
   - `just serve-frontend`

Both `serve-*` commands load `.env`. The backend port is controlled by `GAMMABOARD_BACKEND_PORT`, and the frontend uses `REACT_APP_API_BASE_URL`.

### Live test flows
- `just live-test-basic`
- `just live-test-gammaloop`

Useful stop commands:
- `just stop`
- `just restart-db`

## Manual Flow
1. Add a run:
   - `cargo run --bin gammaboard -- run add configs/live-test-unit-naive-scalar.toml`
2. Start one or more run-nodes:
   - `cargo run --bin gammaboard -- run-node --node-id node-a --poll-ms 1000`
   - `cargo run --bin gammaboard -- run-node --node-id node-b --poll-ms 1000`
3. Assign roles:
   - `cargo run --bin gammaboard -- node assign node-a evaluator <RUN_ID>`
   - `cargo run --bin gammaboard -- node assign node-b sampler-aggregator <RUN_ID>`
4. Start the run:
   - `cargo run --bin gammaboard -- run start <RUN_ID>`

Useful lifecycle commands:
- `cargo run --bin gammaboard -- run pause <RUN_ID>`
- `cargo run --bin gammaboard -- run stop <RUN_ID>`
- `cargo run --bin gammaboard -- run remove <RUN_ID>`
- `cargo run --bin gammaboard -- node stop <NODE_ID>`

## Configuration
Run configuration is TOML and is deep-merged over `configs/default.toml` when you call `gammaboard run add <file.toml>`.

Current top-level structure:
```toml
name = "example"
target = { kind = "scalar", value = 1.23 } # optional

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[sampler_aggregator]
kind = "naive_monte_carlo"

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

Notes:
- `name` is stored in `runs.name`.
- `target` is stored verbatim in `runs.target`.
- `point_spec` is derived from the evaluator during preflight and stored on the run.
- Observable semantics are evaluator-owned. There is no separate `[observable]` section anymore.
- Evaluators that support multiple observable semantics use `observable_kind` inside `[evaluator]`.

Examples:
- `unit`: optional `observable_kind = "scalar" | "complex"`
- `gammaloop`: optional `observable_kind = "scalar" | "complex"`, plus `training_projection`
- `symbolica`: scalar observable semantics only
- `sinc_evaluator`: complex observable semantics only

## Runtime Model
- Batches are queued in PostgreSQL.
- Evaluators claim batches, apply parametrization, evaluate them, and write back `BatchResult`.
- `BatchResult` contains optional training values and a tagged observable payload.
- Sampler-aggregators consume completed batches, merge observable state, ingest training values when needed, and delete consumed completed batches.
- Run and worker performance snapshots are persisted periodically.
- Runtime logs are persisted in `runtime_logs` when DB logging is enabled.

## Logging
- Console logging uses tracing.
- Default console behavior is `INFO` for `gammaboard*` targets and `WARN` for external targets.
- `-q` suppresses all `INFO` output.
- Set `GAMMABOARD_DISABLE_DB_LOGS=1` to disable DB log persistence while keeping console logging.
- DB sink levels are configured with:
  - `GAMMABOARD_DB_LOG_LEVEL`
  - `GAMMABOARD_DB_EXTERNAL_LOG_LEVEL`

## Dashboard/API
Main read APIs:
- `GET /api/runs`
- `GET /api/runs/:id`
- `GET /api/runs/:id/logs`
- `GET /api/runs/:id/aggregated/range`
- `GET /api/runs/:id/performance/evaluator`
- `GET /api/runs/:id/performance/sampler-aggregator`
- `GET /api/workers`
- `GET /api/workers/:id/performance/evaluator`
- `GET /api/workers/:id/performance/sampler-aggregator`

Notes:
- Aggregated history uses sampled range reads with explicit `latest` in the response.
- `BIGINT` identifiers are serialized as strings for frontend safety.
- Observable payloads are tagged JSON, for example `kind = scalar` or `kind = complex`.

## Development
- Rust formatting/checks/tests:
  - `cargo fmt`
  - `cargo check -q`
  - `cargo test -q`
- Frontend:
  - `npm --prefix dashboard test -- --watch=false`
  - `npm --prefix dashboard run build`

If you change architecture, config shape, CLI behavior, or operational workflow, update this file and `AGENTS.md` in the same change.
