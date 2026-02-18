# Gammaboard

Gammaboard is a distributed numerical integration system that uses PostgreSQL as:
- a work queue (`batches`)
- a control plane (`workers`, assignment/lease tables)
- a storage backend for run snapshots and progress (`aggregated_results`, `sampler_states`)

## Current Architecture

### Binaries

- `server` (`src/bin/server.rs`)
  - Read API for runs, queue stats, aggregated snapshots, and SSE updates.
- `control_plane` (`src/bin/control_plane.rs`)
  - Admin CLI for run lifecycle and desired role assignments.
- `worker` (`src/bin/worker.rs`)
  - Node-local reconciler that starts/stops role loops based on control-plane desired assignments.

### Library Modules

- `src/control_plane/mod.rs`
  - Reconciliation loop per node.
  - Manages local evaluator and sampler-aggregator tasks.
  - Handles spin-up/spin-down when assignments change.
- `src/runners/worker.rs`
  - Evaluator runner: claims batches, evaluates, writes results/failures.
- `src/runners/sampler_aggregator.rs`
  - Sampler-aggregator runner: ingests completed batches, produces new batches, persists state and aggregate snapshots.
- `src/stores/pg_store.rs`
  - Postgres implementation of all contracts (run spec, queue, control plane, leases, aggregation).
- `src/contracts/*`
  - Runtime and storage traits, shared domain models.
- `src/engines/test_only.rs`
  - Test-only evaluator/sampler implementations used by `worker --test`.

### Runtime Flow

1. Create a run with integration params.
2. Assign desired roles (`evaluator`, `sampler-aggregator`) to node ids via `control_plane`.
3. Run `worker` on each node id.
4. Each worker reconciles desired state:
   - starts/stops evaluator and sampler-aggregator loops when assignments change
   - heartbeats itself in `workers`
5. Evaluators consume `batches`; sampler-aggregator produces/aggregates and updates run summary.

## Database Schema

### Work Queue and Results

- `runs`
- `batches`
- `aggregated_results`
- `sampler_states`
- views: `run_progress`, `work_queue_stats`

### Control Plane

- `workers`
  - registered worker identities by role and node, includes desired assignment (`desired_run_id`).
- `run_sampler_aggregator_leases`
  - single active sampler-aggregator lease per run.
- `run_evaluator_assignments`
  - one-to-many evaluator assignments per run.

## Configuration

Run integration params are provided via TOML file and passed to `control_plane run-add`.

Example file: `configs/live-test.toml`

```toml
[worker_runner_params]
loop_sleep_ms = 200
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
```

## Local Usage

### Prerequisites

- Rust (edition 2024)
- PostgreSQL 16
- Node.js + npm (for dashboard)
- Docker (optional, for local Postgres)

### Fast Path

- End-to-end backend test:
  - `just live-test`
- End-to-end with frontend:
  - `just live-test-with-frontend`

Useful controls:
- `just stop-workers`
- `just stop-serving`
- `just restart-db`

### Manual CLI Flow

1. Create run:
   - `cargo run --bin control_plane -- run-add --status pending --integration-params-file configs/live-test.toml`
2. Start workers:
   - `cargo run --bin worker -- -t --node-id node-a --poll-ms 1000`
   - `cargo run --bin worker -- -t --node-id node-b --poll-ms 1000`
3. Assign roles:
   - `cargo run --bin control_plane -- assign --node-id node-a --role evaluator --run-id <RUN_ID>`
   - `cargo run --bin control_plane -- assign --node-id node-b --role sampler-aggregator --run-id <RUN_ID>`
4. Start run:
   - `cargo run --bin control_plane -- run-start --run-id <RUN_ID>`

## Current Limitation

`worker` currently requires `-t` / `--test` and uses test-only engines (`src/engines/test_only.rs`).
Non-test engine wiring is intentionally still `todo!()`.
