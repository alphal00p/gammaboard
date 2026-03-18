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

To snapshot the current local database into the gitignored `dump/` directory:
```bash
just dump-db-sql
just dump-db-custom
```

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
target = { kind = "scalar", value = 1.23 } # optional

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[parametrization]
kind = "identity"
```

Optional top-level task queue:
```toml
[[task_queue]]
kind = "sample"
nr_samples = 100000
[task_queue.sampler_aggregator]
kind = "naive_monte_carlo"
[task_queue.parametrization]
kind = "identity"

[[task_queue]]
kind = "sample"
nr_samples = 200000
[task_queue.sampler_aggregator]
kind = "havana_training"
# training budget comes from `nr_samples`

[[task_queue]]
kind = "sample"
nr_samples = 800000
[task_queue.sampler_aggregator]
kind = "havana_inference"
[task_queue.parametrization]
kind = "havana_inference"
[task_queue.parametrization.inner]
kind = "spherical"

[[task_queue]]
kind = "pause"
```

Sample tasks support inheritance. Omitted `sampler_aggregator` and `parametrization` fields inherit from the previous effective sample stage, or from the run's initial integration settings for the first sample task. If `task_queue` is omitted, the run is created idle and no work will be produced until tasks are appended.

Deterministic scan tasks are also supported:
```toml
[[task_queue]]
kind = "image"
display = "complex_hue_intensity" # optional; "auto" is the default
[task_queue.geometry]
offset = [0.0, 0.0]
u_vector = [1.0, 0.0]
v_vector = [0.0, 1.0]
u_linspace = { start = -2.0, stop = 2.0, count = 128 }
v_linspace = { start = -2.0, stop = 2.0, count = 128 }

[[task_queue]]
kind = "plot_line"
display = "complex_components" # optional; "auto" is the default
[task_queue.geometry]
offset = [0.0, 0.0]
direction = [1.0, 0.0]
linspace = { start = -2.0, stop = 2.0, count = 512 }
```

`image` and `plot_line` tasks rasterize deterministic points directly in evaluator space, persist only compact progress history, and render their current result from the full task-local observable state.

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
