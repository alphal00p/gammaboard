# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane, queue, and telemetry store.

## What It Does
- `gammaboard run` manages run creation, pause, and removal.
- `gammaboard node` manages desired worker assignments.
- `gammaboard run-node` reconciles one local process into one active role loop at a time: `evaluator` or `sampler_aggregator`.
- `gammaboard server` serves the dashboard read API.
- The dashboard exposes `Runs`, `Workers`, `Performance`, and `Logs` tabs.

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
   - If `5432` is already in use on your machine, set `DB_PORT` in `.env` (for example `DB_PORT=5433`) and rerun.
2. Start the backend:
   - `just serve-backend`
3. Start the frontend:
   - `just serve-frontend`

Both `serve-*` commands load `.env`. The backend port is controlled by `GAMMABOARD_BACKEND_PORT`, and the frontend uses `REACT_APP_API_BASE_URL`.

### CLI completions and shortcut
- `just build` also creates `~/.cargo/bin/gammaboard` as a symlink to the built binary so you can run `gammaboard ...` directly with the same command name your shell already resolves from Cargo's bin directory.
- Generate shell completions with:
  - `./target/dev-optim/gammaboard completion bash`
  - `./target/dev-optim/gammaboard completion zsh`
  - `./target/dev-optim/gammaboard completion fish`
- The generated script can be sourced directly, or installed through your shell's normal completion directory.

### Live test flows
- `just live-test-basic`
- `just live-test-gammaloop`

Useful stop commands:
- `just stop`
- `just restart-db`
- `just start 8`

## Manual Flow
1. Add a run:
   - `cargo run --bin gammaboard -- run add configs/live-test-unit-naive-scalar.toml`
2. Start one or more run-nodes:
   - `just start 2`
   - Worker IDs are `w-1`, `w-2`, ... in sequence.
3. Assign roles:
   - `cargo run --bin gammaboard -- node assign w-1 evaluator <RUN_ID>`
   - `cargo run --bin gammaboard -- node assign w-2 sampler-aggregator <RUN_ID>`
   - Each run may have many evaluator assignments, but at most one sampler-aggregator assignment.
   - Each node may have at most one desired assignment. Assigning a new role on the same node replaces the previous desired assignment.

Useful lifecycle commands:
- `cargo run --bin gammaboard -- run pause <RUN_ID>`
- `cargo run --bin gammaboard -- run remove <RUN_ID>`
- `cargo run --bin gammaboard -- node list`
- `cargo run --bin gammaboard -- node unassign <NODE_ID>`
- `cargo run --bin gammaboard -- node stop <NODE_ID>`

`gammaboard node list` prints one row per node with `ID / Run / Role / Last Seen`. `run-node` registers the node immediately, so freshly started idle nodes appear with `Run = N/A` and `Role = None`.

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
max_batch_size = 64
max_batches_per_tick = 8
max_queue_size = 128
completed_batch_fetch_limit = 1024

[sampler_aggregator_runner_params.stop_on]
kind = "samples_at_least"
samples = 1000000
```

Notes:
- `name` is stored in `runs.name`.
- `target` is stored verbatim in `runs.target`.
- `point_spec` is derived from the evaluator during preflight and stored on the run.
- Observable semantics are evaluator-owned. There is no separate `[observable]` section anymore.
- Evaluators that support multiple observable semantics use `observable_kind` inside `[evaluator]`.
- Optional `sampler_aggregator_runner_params.stop_on` supports automatic run pause on conditions.
  Current condition support: `kind = "samples_at_least"` with positive integer `samples`.
  `samples_at_least` is evaluated against aggregated observable sample count.
  After threshold is reached, sampler-aggregator stops producing new batches and clears desired assignments for the run.

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
- Sampler-aggregators own any per-batch training correlation state internally; the runner does not persist or return batch context.
- The latest full runtime observable is stored on the run record as `current_observable`.
- On pause/unassignment, the sampler-aggregator finishes its current tick and persists a runner snapshot to `runs.sampler_runner_snapshot` before exiting.
- Resume currently restores sampler snapshots for `naive_monte_carlo` and `havana`, including Havana RNG state.
- `runs.sampler_runner_snapshot` is internal control-plane state and is not exposed by the dashboard read API.
- Run lifecycle is derived from control-plane state rather than persisted on `runs`:
  desired assignments present -> `running`; no desired assignments but active workers or claimed batches remain -> `pausing`; otherwise -> `paused`.
- Desired and current node state live directly on `nodes`; both desired fields and both current fields must be either null together or set together.
- Aggregated observable history snapshots persist the observable's reduced persistent payload rather than the tagged runtime `ObservableState`.
- Run and worker performance snapshots are persisted periodically.
- Evaluator performance history stores generic evaluator metrics only; evaluator-specific static details belong in evaluator init metadata.
- Sampler performance history stores generic sampler metrics plus sampler runtime metrics, while sampler-specific diagnostics remain separate.
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
- Run logs are cursor-paged and server-filtered. `GET /api/runs/:id/logs` accepts `limit`, `worker_id`, `level`, `q`, and `before_id`, and returns `{ items, next_before_id, has_more_older }`.
- `BIGINT` identifiers are serialized as strings for frontend safety.
- Observable payloads are tagged JSON, for example `kind = scalar` or `kind = complex`.
- Run payloads from `GET /api/runs` and `GET /api/runs/:id` include `point_spec` from `runs.point_spec` and the latest full observable as `current_observable`.
- Finished-run dashboard views should use persisted run/history data; live worker payloads are only for active telemetry.
- The `Workers` tab shows live worker assignment/heartbeat/role state; historical evaluator/sampler performance is viewed separately by run and worker.
- The `Performance` tab is run-scoped and worker-scoped, using persisted snapshots. For sampler-aggregators, produce and ingest timing are shown separately, with latest-snapshot summary cards below the charts.
- In the `Runs` tab, the sampler panel should prioritize target-vs-actual runtime values and current performance metrics; low-level runner bounds remain available in the JSON view instead of the summary card.

Dashboard behavior:
- Worker data is polled once at the app level and shared across the `Runs`, `Workers`, and `Logs` tabs.
- The `Logs` tab is intentionally view-only with server-side filters and `Load older` pagination instead of client-side pause/buffer/grid state.

## Development
- Store bootstrap/composition helpers used by runners live under `src/stores/*`.
- HTTP server runtime and handlers live under `src/server/*`; `src/cli/*` wires arguments and startup.
- Rust formatting/checks/tests:
  - `cargo fmt`
  - `cargo check -q`
  - `cargo test -q`
- Frontend:
  - `npm --prefix dashboard test -- --watch=false`
  - `npm --prefix dashboard run build`
- Local worker utility:
  - `just start <N>` starts `N` local `run-node` processes with sequential IDs `w-1` through `w-N`.

If you change architecture, config shape, CLI behavior, or operational workflow, update this file and `AGENTS.md` in the same change.
