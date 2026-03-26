# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane.

## Main Commands
- `gammaboard run`: create, list, pause, clone, and remove runs.
- `gammaboard node`: list, assign, unassign, and stop nodes.
- `gammaboard run-node --name <NODE_NAME>`: start one local worker process.
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

`serve-*` commands load `.env`. The frontend uses `REACT_APP_API_BASE_URL`. The CLI reads its shared database and tracing settings from `configs/gammaboard.toml`, and the backend reads its host, port, auth, cookie, and template settings from `configs/server.toml`.

## CLI Config
- All commands load shared runtime config from [configs/gammaboard.toml](/home/cedricsigrist/Workspace/repos/gammaboard/configs/gammaboard.toml) by default.
- Override it when needed with:
  ```bash
  gammaboard --cli-config path/to/gammaboard.toml <COMMAND>
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
gammaboard db init
gammaboard db start
gammaboard db create
gammaboard db stop
gammaboard db reset
gammaboard db dump-sql
```

These commands use `database.url` and `local_postgres` from `configs/gammaboard.toml`.

## Server Config
- The server is configured from a single TOML file. By default:
  ```bash
  gammaboard server
  ```
- Override the server config path when needed with:
  ```bash
  gammaboard server --server-config path/to/server.toml
  ```
- The checked-in local default is [configs/server.toml](/home/cedricsigrist/Workspace/repos/gammaboard/configs/server.toml).
- First `Ctrl-C` requests graceful shutdown. If draining hangs, press `Ctrl-C` again to force shutdown; the server also force-exits automatically after 10 seconds.
- Required shape:
  ```toml
  host = "0.0.0.0"
  port = 4000
  allowed_origin = "http://localhost:3000"
  secure_cookie = false
  run_templates_dir = "templates/runs"
  task_templates_dir = "templates/tasks"

  [auth]
  admin_password_hash = "$argon2id$..."
  session_secret = "replace-me"
  ```
- All server config fields are explicit; the server does not fill in defaults.

## Dashboard Auth
- Read-only dashboard endpoints stay open.
- Steering actions currently require admin login and are backed by a signed session cookie.
- The dashboard currently supports creating runs from raw TOML, cloning runs from a stored stage snapshot, appending tasks from raw TOML, pausing runs, auto-assigning free nodes, assigning and unassigning nodes, and requesting a node shutdown.
- The create-run and add-task dialogs can also load `.toml` templates from `run_templates_dir` and `task_templates_dir` in `server.toml`.
- Node shutdown from the dashboard is guarded by a confirmation dialog because it cannot be undone from the web UI.
- Put `auth.admin_password_hash` in `server.toml` to enable dashboard auth.
- Put `auth.session_secret` in `server.toml` when auth is enabled.
- Set `allowed_origin` in `server.toml` if the frontend is served from a different origin than `http://localhost:3000`.
- Deploy this behind HTTPS for real use and set `secure_cookie = true` in `server.toml`.
- Generate the password hash with:
  ```bash
  gammaboard auth --password 'your-password'
  ```

`auth.admin_password_hash` should contain the full Argon2 encoded hash output from that command.

## Run Configs
Run configs are TOML and are deep-merged over `configs/default.toml`.

Add a run with:
```bash
gammaboard run add configs/live-test-unit-naive-scalar.toml
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

If `task_queue` is omitted, the run is created idle.
Every run stores an initial root stage snapshot (`sequence_nr = 0`) immediately at creation.

### Task Queue
Sample tasks use either `snapshot_id` (branch from an existing stage snapshot) or `config` (derive from current effective stage with overrides). Do not set both.
Within `config`, omitted `sampler_aggregator` and `batch_transforms` inherit from the previous effective stage. `batch_transforms = []` explicitly clears the inherited transform list.
`config.observable` may be omitted to reuse the previous observable state.
Use `nr_samples = 0` when you want a sample task to only update stage state without producing work.
Task files used with `gammaboard run task add` may contain either a single `task = { ... }`, a `[[task_queue]]` array, or both. When both are present, `task` is appended first.

Executable sample tasks may also branch from an older stage snapshot:
```toml
snapshot_id = 42
```
Relative indices are supported too: `snapshot_id = -1` means "latest prior stage snapshot", `-2` means "second latest prior snapshot", etc.

Sample task config example:
```toml
[[task_queue]]
name = "warmup-sample" # optional; auto-generated when omitted
kind = "sample"
nr_samples = 10000
[task_queue.config]
observable = "scalar"
[task_queue.config.sampler_aggregator]
kind = "naive_monte_carlo"
```

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
The dashboard clone dialog also exposes the initial root snapshot as a selectable clone source.

## Nodes
Start local workers:
```bash
just start 2
```

Or directly:
```bash
gammaboard run-node --name w-1
```

`run-node` uses a fast-start reconcile backoff internally: it starts polling at `100ms`, grows by a factor of `1.1`, and caps at `1s`.

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
