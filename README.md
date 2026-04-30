# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane.

## Main Commands
- `gammaboard run`: create, list, pause, clone, and remove runs.
- `gammaboard node`: list, assign, unassign, and stop nodes.
- `gammaboard server`: run the backend directly in the foreground.
- `gammaboard deploy run`: supervise the full-stack deploy flow in the foreground.
- `gammaboard db`: manage the local PostgreSQL instance used for development.

The dashboard shows runs, task output, nodes, performance, and logs.

## Prerequisites
- Rust
- PostgreSQL 16 tools (`initdb`, `pg_ctl`, `postgres`, `psql`) available locally for `gammaboard db ...`
- `sqlx` CLI: `cargo install sqlx-cli --no-default-features --features postgres`
- Node.js + npm for the frontend

## Local Setup
1. Start PostgreSQL:
   ```bash
   gammaboard db start
   ```
2. Build:
   ```bash
   just build
   ```
3. Start the backend API:
   ```bash
   gammaboard server
   ```
4. Start the frontend:
   ```bash
   just serve-frontend
   ```

Default config split:
- `configs/runtime/default.toml`: shared database, tracing, and local Postgres settings
- `configs/server/default.toml`: direct `gammaboard server` settings
- `ops/<env>/config/{server,deploy}.toml`: deploy environment profiles

The frontend uses relative `/api` calls and does not require `.env`. The `just` recipes remain as thin wrappers, but the CLI flow above is the primary local workflow.

## UBELIX Quickstart
For initial Slurm/Apptainer hello-world tests on UBELIX, use:

- [ops/ubelix/README-ubelix.md](/home/cedricsigrist/Workspace/repos/gammaboard/ops/ubelix/README-ubelix.md)
- [ops/ubelix/ops/slurm/smoke.sbatch](/home/cedricsigrist/Workspace/repos/gammaboard/ops/ubelix/ops/slurm/smoke.sbatch)
- [ops/ubelix/justfile](/home/cedricsigrist/Workspace/repos/gammaboard/ops/ubelix/justfile)

## Ops Layout
- [ops/local/config](/home/cedricsigrist/Workspace/repos/gammaboard/ops/local/config): local deploy profiles.
- [ops/itphlies/README.md](/home/cedricsigrist/Workspace/repos/gammaboard/ops/itphlies/README.md): ITPhlies-specific deploy workflow and config.
- [ops/ubelix/README-ubelix.md](/home/cedricsigrist/Workspace/repos/gammaboard/ops/ubelix/README-ubelix.md): UBELIX Slurm/Apptainer workflow and config.

## Runtime Config
- All commands load shared runtime config from [configs/runtime/default.toml](/home/cedricsigrist/Workspace/repos/gammaboard/configs/runtime/default.toml) by default.
- If that default path is not present on disk, the CLI falls back to the built-in default runtime TOML.
- Override it when needed with:
  ```bash
  gammaboard --runtime-config path/to/runtime/default.toml <COMMAND>
  ```
- Required shape:
  ```toml
  [database]
  url = "postgresql://postgres:password@127.0.0.1:5433/gammaboard_db"

  [tracing]
  persist_runtime_logs = true
  db_gammaboard_level = "info"
  db_external_level = "warn"

  [resources]
  # Optional ordered search roots for relative evaluator resource paths
  # (for example GammaLoop state_folder). First existing match wins.
  roots = []

  [local_postgres]
  data_dir = ".postgres"
  socket_dir = ".postgres-socket"
  log_file = ".postgres/logfile"
  max_connections = 512
  shared_buffers = "4GB"
  effective_cache_size = "32GB"
  work_mem = "64MB"
  checkpoint_timeout = "30min"
  max_wal_size = "8GB"
  wal_compression = true
  synchronous_commit = false
  ```

## Local Postgres Commands
Use the CLI for local database lifecycle:

```bash
gammaboard db status
gammaboard db start
gammaboard db stop
gammaboard db delete
gammaboard db dump-sql
```

These commands use `database.url` and `local_postgres` from `configs/runtime/default.toml`.
Relative evaluator resource paths (for example `evaluator.kind = "gammaloop"` `state_folder`) resolve against `resources.roots` in order; absolute paths are used as-is.
To reset local state, use `just db-reset` or run `gammaboard db delete --yes` then `gammaboard db start`.
`local_postgres.max_connections` controls the local Postgres server connection ceiling used by `gammaboard db start`.
The checked-in local defaults also bias Postgres toward queue throughput: larger buffers/WAL limits, `wal_compression = true`, and `synchronous_commit = false`. That last setting trades crash durability of the most recent transactions for lower write latency, which is a good fit for the transient local batch queue but should be revisited for stricter durability needs.
`gammaboard db start` always enables local query statistics by starting Postgres with `shared_preload_libraries=pg_stat_statements` and running `CREATE EXTENSION IF NOT EXISTS pg_stat_statements;` for the configured database.

## Server Config
- The server is configured from a single TOML file. By default:
  ```bash
  gammaboard server
  ```
- Override the server config path when needed with:
  ```bash
  gammaboard server --server-config path/to/server/default.toml
  ```
- The checked-in local default is [configs/server/default.toml](/home/cedricsigrist/Workspace/repos/gammaboard/configs/server/default.toml).
- If that default path is not present on disk, the CLI falls back to the built-in default server TOML.
- `Ctrl-C` terminates the server process immediately.
- Required shape:
  ```toml
  api_host = "0.0.0.0"
  api_port = 4000
  allowed_origins = ["http://localhost:3000"]
  secure_cookie = false
  allow_db_admin = true
  allow_local_node_spawn = true
  run_templates_dir = "../runs"
  task_templates_dir = "../tasks"

  [auth]
  admin_password_hash = "$argon2id$..."
  session_secret = "replace-me"
  ```
- All server config fields are explicit; the server does not fill in defaults.
- Set `allow_db_admin = true` only for trusted local/operator setups; it enables dashboard-triggered `db stop && db start`.
- Set `allow_local_node_spawn = false` for scheduler-managed deployments (for example UBELIX). Dashboard node-start actions still create DB-backed launch requests, but an external launcher must resolve them.
- `gammaboard server` is the direct local/manual backend path. Use `gammaboard deploy run ...` when you want one foreground process to supervise local Postgres, the backend, and nginx-backed frontend serving.

## Deploy Config
Deploy is owned by `gammaboard deploy run ...` plus a deploy TOML profile.
- `gammaboard deploy` default config (`ops/local/config/deploy.toml`) also has the same built-in fallback when the default file is absent.

Config split:
- `configs/runtime/*.toml`: shared DB, tracing, and local Postgres settings for all commands
- `configs/server/default.toml`: shared direct `gammaboard server` default profile
- `ops/<env>/config/server.toml`: environment-specific deploy backend settings
- `ops/<env>/config/deploy.toml`: environment-specific deploy orchestration (server profile, frontend HTTP exposure, static frontend serving, cleanup policy)

The checked-in profiles are:
- [ops/local/config/deploy.toml](/home/cedricsigrist/Workspace/repos/gammaboard/ops/local/config/deploy.toml)
- [ops/itphlies/config/deploy.toml](/home/cedricsigrist/Workspace/repos/gammaboard/ops/itphlies/config/deploy.toml)

Use:
```bash
gammaboard deploy run --deploy-config ops/local/config/deploy.toml
```

Useful options:
- `--frontend-port <PORT>` overrides the deploy profile's frontend/nginx listen port for that launch

Deploy profiles now derive the printed/open URLs from `frontend_http.frontend_port` plus `frontend_http.frontend_advertise_hosts`, instead of duplicating full URL strings in the config.

The `just` wrappers build first, then run the foreground supervisor:
```bash
just deploy local dev
just deploy itphlies release
```

Deploy run:
- optionally starts local Postgres via `gammaboard db start`
- starts `gammaboard server` as a supervised child process using the same active `--runtime-config` path
- generates an nginx config from the deploy profile and runs nginx in the foreground as a supervised child process
- logs to the parent process stdout/stderr, so terminals, Slurm, or systemd own log collection
- tears down nginx, the backend, worker assignments, and local Postgres on `Ctrl-C`/`SIGTERM`

`gammaboard deploy run ...` itself does not build. Use the `just deploy ...` wrapper when you want the frontend and backend built first.

## ITPhlies Deployment
Use this flow when you want both direct LAN access and the SSH tunnel option.

1. On ITPhlies, from the repo root, run:
   ```bash
   just deploy itphlies release
   ```
   (From `ops/itphlies`, you can run `just --justfile justfile deploy`.)
   This builds the frontend and release backend, then runs a foreground deploy supervisor for the backend, nginx, and local Postgres.
2. On your laptop, open an SSH tunnel:
   ```bash
   ssh -N -L 8080:127.0.0.1:8080 ITPhliesTails
   ```
3. Open either:
   ```text
   http://localhost:8080
   ```
   or `http://itphlies:8080` if your local network resolves that hostname. If you access the server by LAN IP instead, add that origin to `allowed_origins` in the server config first.
4. To stop all deployed ITPhlies processes, press `Ctrl-C` in the foreground deploy terminal.
5. The SSH tunnel remains optional; direct LAN access works because nginx listens on `0.0.0.0:8080`, while the backend still stays private on `127.0.0.1:4000`.
6. Interactive deploy profiles disable nginx access logs by default to keep foreground output readable. Re-enable them in the deploy TOML only when debugging HTTP traffic.

Config files used:
- backend: [ops/itphlies/config/server.toml](/home/cedricsigrist/Workspace/repos/gammaboard/ops/itphlies/config/server.toml)
- deploy: [ops/itphlies/config/deploy.toml](/home/cedricsigrist/Workspace/repos/gammaboard/ops/itphlies/config/deploy.toml)

Important:
- `ops/itphlies/config/server.toml` currently allows `http://localhost:8080` and `http://itphlies:8080`.
- `ops/itphlies/config/deploy.toml` advertises `localhost` and `itphlies` as the operator-facing URLs for that deploy profile.
- If you want to access the UI via a raw LAN IP or another hostname, add that exact origin to `allowed_origins`.
- Backend listens on `127.0.0.1:4000`; nginx listens on `0.0.0.0:8080`.
- ITPhlies deployment uses the release backend binary by default.

## Frontend API Routing
- The dashboard frontend always calls relative `/api` endpoints.
- Local dev: `dashboard/package.json` sets `"proxy": "http://127.0.0.1:4000"` so `npm start` forwards `/api/*` to the backend.
- Production: serve frontend and backend behind the same origin, and route `/api/*` to `gammaboard server` via your reverse proxy.
- Example nginx layout:
  - `location / { root <dashboard-build-dir>; try_files $uri /index.html; }`
  - `location /api/ { proxy_pass http://127.0.0.1:4000/api/; }`
- Local deploy setup:
  - server config: [ops/local/config/server.toml](/home/cedricsigrist/Workspace/repos/gammaboard/ops/local/config/server.toml)
  - deploy config: [ops/local/config/deploy.toml](/home/cedricsigrist/Workspace/repos/gammaboard/ops/local/config/deploy.toml)
  - run with: `just deploy local dev`
  - stop with: `Ctrl-C` in the deploy terminal

## Dashboard Auth
- Read-only dashboard endpoints stay open.
- Steering actions currently require admin login and are backed by a signed session cookie.
- The Runs tab includes a `Copy Run TOML` action that exports a run-add compatible TOML containing the run config and all successfully completed tasks as `[[task_queue]]` entries.
- The dashboard currently supports creating runs from raw TOML, cloning runs from a stored stage snapshot, appending tasks from raw TOML, deleting pending tasks, pausing runs, removing runs, auto-assigning free nodes, assigning and unassigning nodes, requesting node shutdown (single or all), creating grouped node launch requests, resetting the local database when `allow_db_admin = true`, and shutting down the control process.
- When `allow_local_node_spawn = true`, the server resolves node launch requests by spawning local child processes. Otherwise launch requests remain queued for an external launcher.
- The Performance tab defaults history x-axes to sampler uptime (active sampler runtime, excluding paused intervals) and lets operators switch to wall time or total completed samples.
- The create-run and add-task dialogs can load `.toml` templates from `run_templates_dir` and `task_templates_dir`; admin users can also save edited TOML back as templates, and task templates can be deleted from the dashboard.
- Node shutdown from the dashboard is guarded by a confirmation dialog.
- Put `auth.admin_password_hash` in your server config to enable dashboard auth.
- Put `auth.session_secret` in your server config when auth is enabled.
- Set `allowed_origins` in your server config if the frontend is served from origins other than `http://localhost:3000`.
- Deploy this behind HTTPS for real use and set `secure_cookie = true` in your server config.
- Generate the password hash with:
  ```bash
  gammaboard auth --password 'your-password'
  ```

`auth.admin_password_hash` should contain the full Argon2 encoded hash output from that command.

## Run Configs
Run configs are TOML and are deep-merged over the built-in default run config template (mirrors `configs/runs/default.toml`).

Add a run with:
```bash
gammaboard run add configs/runs/gammaloop.toml
```

Flake-backed Python evaluator + sampler example:
```bash
gammaboard run add configs/runs/python-scalar-python-sampler-flake-demo.toml
```

Curated run configs (kept intentionally small):
- `configs/runs/default.toml`: baseline defaults merged into every run config.
- `configs/runs/symbolica-havana-pdf-1d2d.toml`: Symbolica + Havana training + both PDF adaptation task kinds (`pdf_adaptation_image`, `pdf_adaptation_plot_line`).
- `configs/runs/python-scalar-python-sampler-flake-demo.toml`: Python evaluator and Python sampler integration.
- `configs/runs/gammaloop.toml`: GammaLoop TTH evaluator config, including optional `post_load_commands`.

Curated task bundles:
- `configs/tasks/sample_monte_carlo_real.toml`: minimal scalar sample task with naive Monte Carlo.
- `configs/tasks/pdf_adaptation_image.toml`: Havana training followed by PDF adaptation image rasterization.
- `configs/tasks/train_sample.toml`: GammaLoop TTH train+sample queue with queue tuning and inference stop target (`relative_error = 0.001`, `max_samples = 1_000_000_000`).

Minimal shape:
```toml
name = "example"
target = { kind = "scalar", value = 1.23 } # optional
# or: target = { kind = "complex", re = 1.23, im = 0.0 }

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
```

For `evaluator.kind = "gammaloop"`, `continuous_dims` and `discrete_dims` are inferred from the selected integrand and should be omitted.
Gammaboard evaluates GammaLoop runs in x-space so GammaLoop's parameterized observable and histogram path is used.
`post_load_commands = ["set ...", ...]` is optional and executes in-memory after loading the GammaLoop state and before integrand selection; commands are not saved back to disk.

For `evaluator.kind = "python_scalar"`, configure:
- `flake_ref`: nix flake reference that resolves to a runtime package (for example `path:./python_api/examples/python_scalar_sin#runtime`)
- `module`: python module name to import
- `class`: class name to instantiate (must expose `eval(xs_discrete, xs_continuous)`)
- `continuous_dims`: expected continuous dimension for homogeneous rectangular batches
- `discrete_dims`: expected discrete dimension for homogeneous rectangular batches
- optional `init_args = { ... }`: constructor/config payload forwarded to python init
- Optional evaluator ABCs are provided in `python_api.evaluator` (`ScalarBatchIntegrand`, `ComplexBatchIntegrand`) for type-checkable interfaces.

Python construction semantics:
- if the class defines `from_config(discrete_dims=..., continuous_dims=..., init_args=...)`, that is called
- otherwise the worker calls `ClassName(**init_args)` (or `ClassName()` when `init_args` is empty)

For `sampler_aggregator.kind = "python_homogeneous_monte_carlo"`, configure:
- `flake_ref`: nix flake reference that resolves to a runtime package
- `module`: python module name to import
- `class`: python class implementing sampler methods (`sample_plan`, `training_samples_remaining`, `produce_latent_batch`, `ingest_training_weights`, `snapshot`, optional `pdf`)
- `continuous_dims`: expected homogeneous continuous dimension
- `discrete_dims`: expected homogeneous discrete dimension
- optional `init_args = { ... }`: constructor/config payload forwarded to python init
- Optional sampler ABC is provided in `python_api.sampler` (`SamplerAggregator`) for a typed contract.
- Python evaluator and sampler methods are vectorized over fixed rectangular batches:
  `xs_discrete` has shape `(nr_samples, discrete_dims)` and `xs_continuous` has shape `(nr_samples, continuous_dims)`.
- `produce_latent_batch(nr_samples)` returns `(xs_discrete, xs_continuous)`.
- optional `pdf(xs_discrete, xs_continuous)` returns `(nr_samples,)`.

Python sampler construction semantics:
- restore path: `from_snapshot(snapshot=..., discrete_dims=..., continuous_dims=..., init_args=...)` when present
- fresh path: `from_config(discrete_dims=..., continuous_dims=..., init_args=...)` when present
- fallback: `ClassName(**init_args)` / `ClassName()`
- Worker protocol entrypoints are checked in under `python_api/python_workers/` and launched with the flake runtime python executable.

If `task_queue` is omitted, the run is created idle.
Every run stores an initial root stage snapshot (`sequence_nr = 0`) immediately at creation.

### Task Queue
Sample tasks use direct per-component source specs:
- omit `sampler_aggregator` or `accumulator` to use `latest`
- use `{ from_name = "..." }` to load from a prior task name
- use `{ config = ... }` to set explicit inline config
- `accumulator = { config = "gammaloop" }` is available for GammaLoop runs and persists GammaLoop's native histogram snapshot bundle

Task names are unique per run and can be referenced by `from_name`.
`batch_transforms` is stage state for tasks. Omitted inherits; `batch_transforms = []` explicitly clears inherited transforms.
When you want raster `image`/`plot_line`/`pdf_adaptation_image`/`pdf_adaptation_plot_line` tasks to evaluate directly in declared geometry coordinates after transformed sampling stages, set `batch_transforms = []` on those raster tasks.
Use `stop_condition = { max_samples = 0 }` when you want a sample task to only update stage state without producing work. This is the configuration-only task shape.
Task files used with `gammaboard run task add` may contain either a single `task = { ... }`, a `[[task_queue]]` array, or both. When both are present, `task` is appended first.

Sample task config example:
```toml
[[task_queue]]
name = "warmup-sample" # optional; auto-generated when omitted
kind = "sample"
stop_condition = { max_samples = 10000, absolute_error = 1e-3, relative_error = 1e-2, projection = "real" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
```

Evaluators use a fixed single-slot latent prefetch and single-slot async submit pipeline. Materialization and evaluation still remain strictly one batch at a time.

`sampler_aggregator_runner_params` also controls queue and persistence behavior:
- `frontend_sync_interval_ms` sets how often the sampler runner refreshes the frontend-facing accumulator state and persisted accumulator snapshots during sampling; the shared run default is `2000`.
- Sampler queue settings live under the nested TOML table `[sampler_aggregator_runner_params.queue]`.
- `target_batch_eval_ms`, `batch_size_deadband_ratio`, `batch_size_cooldown_ticks`, and `max_batch_size` are queue-level controls under `[sampler_aggregator_runner_params.queue]`; the queue owns batch-size tuning during production planning.
- `queue_buffer` is the single queue buffer knob for the sampler queue. The runner targets about `queue_buffer * active_evaluator_count` pending batches. A value of `0.0` is the most aggressive and lets the queue drain to zero pending batches when the sampler cannot refill it faster than evaluators consume work. Larger values keep more pending work buffered. `max_queue_size` remains the hard cap.
- Refill hysteresis is controlled with `pending_refill_low_ratio` and `pending_refill_high_ratio`. Production starts when pending drops to the low watermark and refills toward the high watermark.
- Total open batches (`pending + claimed + completed`) are still capped by `max_queue_size`.
- The sampler queue owns a local pending-insert buffer and a local processed buffer. Polling completed work is non-blocking during normal ticks, and pause/unassign drains the local queue fully before the sampler checkpoint is persisted.

Deterministic scan tasks are supported:
```toml
[[task_queue]]
kind = "image"
accumulator = "complex"
[task_queue.geometry]
offset = [0.0, 0.0]
u_vector = [1.0, 0.0]
v_vector = [0.0, 1.0]
u_linspace = { start = -2.0, stop = 2.0, count = 128 }
v_linspace = { start = -2.0, stop = 2.0, count = 128 }

[[task_queue]]
kind = "plot_line"
accumulator = "complex"
[task_queue.geometry]
offset = [0.0, 0.0]
direction = [1.0, 0.0]
linspace = { start = -2.0, stop = 2.0, count = 512 }

[[task_queue]]
kind = "pdf_adaptation_plot_line"
[task_queue.geometry]
offset = [0.0, 0.0]
direction = [1.0, 0.0]
linspace = { start = -2.0, stop = 2.0, count = 512 }
# sampler_aggregator omitted => latest; or use { from_name = "..." }
```

## Runs And Names
- Run names are not unique.
- CLI run arguments accept either a numeric id or an exact name.
- If a name matches multiple runs, the CLI prints the matches and asks for an id.

List runs:
```bash
gammaboard run list
gammaboard run list my-run-name
```

Clone a run branch from a specific stage snapshot:
```bash
gammaboard run clone <SOURCE_RUN> <FROM_SNAPSHOT_ID> <NEW_NAME>
```
Clone creates a new run rooted at that snapshot and does not copy queued tasks from the source run.
In the dashboard, clone source is inferred from the selected task (falling back to the run root snapshot).

## Nodes
Start local workers:
```bash
gammaboard node auto-run 2
```

Or directly:
```bash
gammaboard node run --name w-1
```

`node run` uses a fast-start reconcile backoff internally: it starts polling at `50ms`, grows by a factor of `2.0`, and caps at `2s`.
`node run` exits on `Ctrl-C` and `SIGTERM`, and expires its lease on shutdown so the same node name can be reused immediately.
`node auto-run N` picks names `w-1`, `w-2`, ... and skips names that already exist in the control plane.
`node auto-run` uses a moderate default `--db-pool-size 4` to reduce worker-side pool contention while still keeping large fanout manageable under database connection pressure.
Auto-run workers now write per-node startup logs to `logs/nodes/<NODE_NAME>.stdout.log` and `logs/nodes/<NODE_NAME>.stderr.log`.
If an auto-run child exits unsuccessfully, the parent control process logs the exit status together with those log paths.

Node names are unique operator handles. Each live worker also owns an internal UUID lease in PostgreSQL. If the worker cannot re-announce itself for 30 seconds, it shuts down.

Assign roles:
```bash
gammaboard node assign w-1 evaluator <RUN>
gammaboard node assign w-2 sampler-aggregator <RUN>
```

Auto-assign currently free nodes:
```bash
gammaboard auto-assign <RUN> [MAX_EVALUATORS]
```

## Common Commands
```bash
gammaboard run list [RUN_NAME]
gammaboard run pause <RUN>
gammaboard run clone <SOURCE_RUN> <FROM_SNAPSHOT_ID> <NEW_NAME>
gammaboard run task list <RUN>
gammaboard run task add <RUN> <TASK_FILE.toml>
gammaboard run task remove <RUN> <TASK_ID>
gammaboard run remove <RUN>

gammaboard node list
gammaboard node run --name <NODE_NAME>
gammaboard node auto-run <COUNT>
gammaboard node assign <NODE_NAME> <ROLE> <RUN>
gammaboard node unassign <NODE_NAME>
gammaboard node stop <NODE_NAME>
```

## Useful Local Commands
```bash
just stop
just live-test-basic
just live-test-gammaloop
```
