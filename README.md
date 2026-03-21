# Gammaboard

Gammaboard runs distributed numerical integration jobs with PostgreSQL as the shared control plane.

## Main Commands
- `gammaboard run`: create, list, pause, clone, and remove runs.
- `gammaboard node`: list, assign, unassign, and stop nodes.
- `gammaboard run-node --name <NODE_NAME>`: start one local worker process.
- `gammaboard server`: start the dashboard backend.

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

`serve-*` commands load `.env`. The backend port is controlled by `GAMMABOARD_BACKEND_PORT`. The frontend uses `REACT_APP_API_BASE_URL`.

## Dashboard Auth
- Read-only dashboard endpoints stay open.
- Steering actions currently require admin login and are backed by a signed session cookie.
- The dashboard currently supports pausing runs, auto-assigning free nodes, assigning and unassigning nodes, and requesting a node shutdown.
- Node shutdown from the dashboard is guarded by a confirmation dialog because it cannot be undone from the web UI.
- Set `GAMMABOARD_ADMIN_PASSWORD_HASH` to enable dashboard auth.
- Set `GAMMABOARD_SESSION_SECRET` when auth is enabled.
- Set `GAMMABOARD_ALLOWED_ORIGIN` if the frontend is served from a different origin than `http://localhost:3000`.
- Deploy this behind HTTPS for real use. Set `GAMMABOARD_SECURE_COOKIE=1` when serving over HTTPS.
- Generate the password hash with:
  ```bash
  gammaboard auth --password 'your-password'
  ```

`GAMMABOARD_ADMIN_PASSWORD_HASH` should contain the full Argon2 encoded hash output from that command.

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
observable = "scalar" # optional

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[parametrization]
kind = "identity"
```

If `task_queue` is omitted, the run is created idle.

### Task Queue
Sample tasks may inherit omitted `sampler_aggregator` and `parametrization` fields from the previous effective sample stage.

Executable tasks may also branch from an older task snapshot:
```toml
start_from = { run_id = 7, task_id = 42 }
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

Clone a run branch from a specific task snapshot:
```bash
gammaboard run clone <SOURCE_RUN> <FROM_TASK_ID> <NEW_NAME>
```

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
gammaboard run clone <SOURCE_RUN> <FROM_TASK_ID> <NEW_NAME>
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
just restart-db
just live-test-basic
just live-test-gammaloop
```
