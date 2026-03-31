# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane.

## Main Commands
- `gammaboard run`: create, list, pause, clone, and remove runs.
- `gammaboard node`: list, assign, unassign, and stop nodes.
- `gammaboard server`: start the dashboard backend.
- `gammaboard db`: manage the local PostgreSQL instance used for development.

The dashboard shows runs, task output, nodes, performance, and logs.

## Prerequisites
- Rust
- PostgreSQL 16
- `sqlx` CLI: `cargo install sqlx-cli --no-default-features --features postgres`
- Node.js + npm for the frontend

## Local Setup
1. Start PostgreSQL:
   ```bash
   just start-db
   ```
2. Build:
   ```bash
   just build
   ```
3. Start the backend:
   ```bash
   just serve-backend
   ```
4. Start the frontend:
   ```bash
   just serve-frontend
   ```

The CLI reads its shared database and tracing settings from `configs/cli/default.toml`, and the backend reads its host, port, auth, cookie, and template settings from `configs/server/default.toml`. The frontend uses relative `/api` calls and does not require `.env`.

## CLI Config
- All commands load shared runtime config from [configs/cli/default.toml](/home/cedricsigrist/Workspace/repos/gammaboard/configs/cli/default.toml) by default.
- Override it when needed with:
  ```bash
  gammaboard --cli-config path/to/cli/default.toml <COMMAND>
  ```
- Required shape:
  ```toml
  [database]
  url = "postgresql://postgres:password@127.0.0.1:5433/gammaboard_db"

  [tracing]
  persist_runtime_logs = true
  db_gammaboard_level = "info"
  db_external_level = "warn"

  [local_postgres]
  data_dir = ".postgres"
  socket_dir = ".postgres-socket"
  log_file = ".postgres/logfile"
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

These commands use `database.url` and `local_postgres` from `configs/cli/default.toml`.
To reset local state: run `gammaboard db delete --yes` then `gammaboard db start`.

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
- `Ctrl-C` terminates the server process immediately.
- Required shape:
  ```toml
  host = "0.0.0.0"
  port = 4000
  allowed_origins = ["http://localhost:3000"]
  secure_cookie = false
  allow_db_admin = true
  run_templates_dir = "../runs"
  task_templates_dir = "../tasks"

  [auth]
  admin_password_hash = "$argon2id$..."
  session_secret = "replace-me"
  ```
- All server config fields are explicit; the server does not fill in defaults.
- Set `allow_db_admin = true` only for trusted local/operator setups; it enables dashboard-triggered `db stop && db start`.

## ITPhlies Deployment
Use this flow when you want both direct LAN access and the SSH tunnel option.

1. On ITPhlies, from the repo root, run:
   ```bash
   just deploy-itphlies
   ```
   This builds the backend in release mode and launches `target/release/gammaboard`.
2. On your laptop, open an SSH tunnel:
   ```bash
   ssh -N -L 8080:127.0.0.1:8080 ITPhliesTails
   ```
3. Open either:
   ```text
   http://localhost:8080
   ```
   or `http://itphlies:8080` if your local network resolves that hostname. If you access the server by LAN IP instead, add that origin to `allowed_origins` in the server config first.
4. To stop all deployed ITPhlies processes:
   ```bash
   just stop-itphlies-deploy
   ```
5. The SSH tunnel remains optional; direct LAN access works because nginx listens on `0.0.0.0:8080`, while the backend still stays private on `127.0.0.1:4000`.

Config files used:
- backend: [configs/server/itphlies-prod.toml](/home/cedricsigrist/Workspace/repos/gammaboard/configs/server/itphlies-prod.toml)
- nginx: [configs/nginx/itphlies-tunnel.conf](/home/cedricsigrist/Workspace/repos/gammaboard/configs/nginx/itphlies-tunnel.conf)

Important:
- `configs/server/itphlies-prod.toml` currently allows `http://localhost:8080` and `http://itphlies:8080`.
- If you want to access the UI via a raw LAN IP or another hostname, add that exact origin to `allowed_origins`.
- Backend listens on `127.0.0.1:4000`; nginx listens on `0.0.0.0:8080`.
- `just deploy-itphlies-server` writes backend PID/log to `logs/itphlies-backend.pid` and `logs/itphlies-backend.log`.
- ITPhlies deployment uses the release backend binary; local dev/local-prod flows still use the `dev-optim` profile.

## Frontend API Routing
- The dashboard frontend always calls relative `/api` endpoints.
- Local dev: `dashboard/package.json` sets `"proxy": "http://127.0.0.1:4000"` so `npm start` forwards `/api/*` to the backend.
- Production: serve frontend and backend behind the same origin, and route `/api/*` to `gammaboard server` via your reverse proxy.
- Example nginx layout:
  - `location / { root <dashboard-build-dir>; try_files $uri /index.html; }`
  - `location /api/ { proxy_pass http://127.0.0.1:4000/api/; }`
- Local production-like test setup:
  - nginx config: [configs/nginx/local-prod.conf](/home/cedricsigrist/Workspace/repos/gammaboard/configs/nginx/local-prod.conf)
  - server config: [configs/server/local-prod.toml](/home/cedricsigrist/Workspace/repos/gammaboard/configs/server/local-prod.toml)
  - run with: `just deploy-local-prod` (serves at `http://localhost:8080`)
  - stop with: `just stop-local-prod`

## Dashboard Auth
- Read-only dashboard endpoints stay open.
- Steering actions currently require admin login and are backed by a signed session cookie.
- The dashboard currently supports creating runs from raw TOML, cloning runs from a stored stage snapshot, appending tasks from raw TOML, deleting pending tasks, pausing runs, removing runs, auto-assigning free nodes, assigning and unassigning nodes, requesting node shutdown (single or all), and starting new local nodes.
- The dashboard currently supports creating runs from raw TOML, cloning runs from a stored stage snapshot, appending tasks from raw TOML, deleting pending tasks, pausing runs, removing runs, auto-assigning free nodes, assigning and unassigning nodes, requesting node shutdown (single or all), starting new local nodes, and restarting the local database when enabled.
- The create-run and add-task dialogs can also load `.toml` templates from `run_templates_dir` and `task_templates_dir` in `server/default.toml`.
- Node shutdown from the dashboard is guarded by a confirmation dialog.
- Put `auth.admin_password_hash` in `server/default.toml` to enable dashboard auth.
- Put `auth.session_secret` in `server/default.toml` when auth is enabled.
- Set `allowed_origins` in `server/default.toml` if the frontend is served from origins other than `http://localhost:3000`.
- Deploy this behind HTTPS for real use and set `secure_cookie = true` in `server/default.toml`.
- Generate the password hash with:
  ```bash
  gammaboard auth --password 'your-password'
  ```

`auth.admin_password_hash` should contain the full Argon2 encoded hash output from that command.

## Run Configs
Run configs are TOML and are deep-merged over `configs/runs/default.toml`.

Add a run with:
```bash
gammaboard run add configs/runs/live-test-unit-naive-scalar.toml
```

Minimal shape:
```toml
name = "example"
target = { kind = "scalar", value = 1.23 } # optional

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
```

For `evaluator.kind = "gammaloop"`, `continuous_dims` and `discrete_dims` are inferred from the selected integrand and should be omitted.

If `task_queue` is omitted, the run is created idle.
Every run stores an initial root stage snapshot (`sequence_nr = 0`) immediately at creation.

### Task Queue
Sample tasks use direct per-component source specs:
- omit `sampler_aggregator` or `observable` to use `latest`
- use `{ from_name = "..." }` to load from a prior task name
- use `{ config = ... }` to set explicit inline config
- `observable = { config = "gammaloop" }` is available for GammaLoop runs and persists GammaLoop's native histogram snapshot bundle

Task names are unique per run and can be referenced by `from_name`.
`batch_transforms` is stage state for tasks. Omitted inherits; `batch_transforms = []` explicitly clears inherited transforms.
When you want raster `image`/`plot_line` tasks to evaluate directly in declared geometry coordinates after transformed sampling stages, set `batch_transforms = []` on those raster tasks.
Use `nr_samples = 0` when you want a sample task to only update stage state without producing work. This is the configuration-only task shape.
Task files used with `gammaboard run task add` may contain either a single `task = { ... }`, a `[[task_queue]]` array, or both. When both are present, `task` is appended first.

Sample task config example:
```toml
[[task_queue]]
name = "warmup-sample" # optional; auto-generated when omitted
kind = "sample"
nr_samples = 10000
observable = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
```

`sampler_aggregator_runner_params` also controls queue and persistence behavior:
- `aggregation_persist_interval_ms` sets how often merged sample observables are flushed to PostgreSQL during training; default is `1000`.
- `target_queue_remaining` is the fraction of runnable in-flight batches (`pending + claimed`) the sampler tries to leave after one tick of evaluator drain. Total open batches (`pending + claimed + completed`) are still capped by `max_queue_size`.
- After the forced initial small batch round-trip, the sampler warms up queue depth conservatively and doubles the target batch count each tick until real drain history is available.
- `strict_batch_ordering` controls whether completed batches are ingested only as a contiguous id prefix (`true`) or in any completed id order (`false`).

Deterministic scan tasks are supported:
```toml
[[task_queue]]
kind = "image"
observable = "complex"
[task_queue.geometry]
offset = [0.0, 0.0]
u_vector = [1.0, 0.0]
v_vector = [0.0, 1.0]
u_linspace = { start = -2.0, stop = 2.0, count = 128 }
v_linspace = { start = -2.0, stop = 2.0, count = 128 }

[[task_queue]]
kind = "plot_line"
observable = "complex"
[task_queue.geometry]
offset = [0.0, 0.0]
direction = [1.0, 0.0]
linspace = { start = -2.0, stop = 2.0, count = 512 }
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
`node auto-run` uses a smaller default `--db-pool-size 2` so large fanout is less likely to fail immediately on database connection pressure.
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
