# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane.
Samplers queue versioned latent batches that evaluators materialize locally through the parametrization layer.

## What It Does
- `gammaboard run` creates, pauses, and removes runs.
- `gammaboard node` assigns or unassigns nodes to runs.
- `gammaboard run-node` starts one local worker process that reconciles into either an `evaluator` or `sampler_aggregator` role.
- `gammaboard server` starts the backend used by the dashboard.
- The dashboard shows runs, nodes, performance, and logs.

## Install

### Prerequisites
- Rust
- PostgreSQL 16
- `sqlx` CLI: `cargo install sqlx-cli --no-default-features --features postgres`
- Node.js + npm for the dashboard
- Docker optional for local Postgres via `docker-compose`

### Build
```bash
just build
```

This also refreshes `~/.cargo/bin/gammaboard` as a symlink to the local build.

### Local setup
1. Start Postgres:
   ```bash
   just start-db
   ```
2. Start the backend:
   ```bash
   just serve-backend
   ```
3. Start the frontend:
   ```bash
   just serve-frontend
   ```

If `5432` is already in use, set `DB_PORT` in `.env` first. `serve-*` commands load `.env`. The backend port is controlled by `GAMMABOARD_BACKEND_PORT`, and the frontend uses `REACT_APP_API_BASE_URL`.

### Shell completion
Install local bash completions with:
```bash
just install-completions
```

Or print completion scripts directly with:
```bash
gammaboard completion <shell>
```

## Use

### Add a run
Run configs are TOML and are deep-merged over `configs/default.toml`.

```bash
gammaboard run add configs/live-test-unit-naive-scalar.toml
```

Minimal config shape:
```toml
name = "example"
pause_on_samples = 1000000 # optional
target = { kind = "scalar", value = 1.23 } # optional

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[sampler_aggregator]
kind = "naive_monte_carlo"

[parametrization]
kind = "identity"
```

Optional top-level task queue:
```toml
[[task_queue]]
kind = "sample"
nr_samples = 100000

[[task_queue]]
kind = "reconfigure_parametrization"
[task_queue.config]
kind = "spherical"

[[task_queue]]
kind = "pause"
```

If `task_queue` is omitted, `run add` creates a default queue from `pause_on_samples`.

### Start local workers
```bash
just start 2
```

That starts `w-1`, `w-2`, and so on.

### Assign roles
```bash
gammaboard node assign w-1 evaluator <RUN_ID>
gammaboard node assign w-2 sampler-aggregator <RUN_ID>
```

Or auto-assign currently free nodes:
```bash
gammaboard auto-assign <RUN_ID> [MAX_EVALUATORS]
```

### Common commands
```bash
gammaboard node assign <NODE_ID> <ROLE> <RUN_ID>
gammaboard node list
gammaboard node unassign <NODE_ID>
gammaboard node stop <NODE_ID>
gammaboard auto-assign <RUN_ID> [MAX_EVALUATORS]
gammaboard run pause <RUN_ID>
gammaboard run task list <RUN_ID>
gammaboard run task add <RUN_ID> <TASK_FILE.toml>
gammaboard run task remove <RUN_ID> <TASK_ID>
gammaboard run remove <RUN_ID>
```

### Useful local commands
```bash
just stop
just restart-db
just live-test-basic
just live-test-gammaloop
```
