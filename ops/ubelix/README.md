# UBELIX Deployment

Current model:
- build `gammaloop` and `gammaboard` as separate Apptainer images
- run control/UI as one Slurm job
- run workers as separate Slurm jobs against the control job database
- access the frontend through one SSH tunnel

Execution context:
- run `python ubelix.py up` on an UBELIX login node
- `python ubelix.py up` prints the SSH tunnel command to run on your local machine
- run `just --justfile ops/ubelix/justfile sync-ops` and `sync-dashboard` on your local machine

## Layout

Repo source lives under `ops/ubelix/` and mirrors this server-side layout directly:

```text
<WORKSPACE_ROOT>/
  ops/
    build/
    config/
    slurm/
  ubelix.py
  justfile
  README.md
```

Generated data typically lives here:

```text
<WORKSPACE_ROOT>/
  artifacts/
    bin/
    dashboard-build/
    migrations/
    sqlx-root/
    src/
    target/
  db/
    <deploy-name>/
  images/
    gammaboard/
    gammaloop/
  logs/
    postgres/
    slurm/
      build/
      control/
      workers/
  runtime/
    runtime-<jobid>-<mode>.toml
  states/
```

## Important Files

- `build/gammaloop.sbatch`
- `build/gammaboard.sbatch`
- `slurm/smoke.sbatch`
- `slurm/single_node_deploy.sbatch`
- `slurm/control.sbatch`
- `slurm/worker.sbatch`
- `ubelix.py`
- `config/runtime.template.toml`
- `config/server.toml`
- `config/server-single-node.toml`
- `config/deploy.toml`
- `config/deploy-single-node.toml`

Notes:
- UBELIX runtime templates set `resources.roots = ["${WORKSPACE_ROOT}/states"]`.
- The UBELIX server profile sets `allow_local_node_spawn = false`.
- Runtime TOML is rendered per job via `envsubst`; control jobs set a loopback database URL, workers receive the control-node database URL from `ubelix.py`.
- Control and single-node deploy jobs use foreground `exec apptainer ... gammaboard deploy run`, so `gammaboard` receives Slurm termination directly. Worker jobs still supervise a sidecar control-job watcher and trap `SIGTERM`/`SIGINT` for graceful cleanup.

## Sync

Run these on your local machine.

Upload ops files:

```bash
just --justfile ops/ubelix/justfile sync-ops
```

Build the frontend locally and upload it:

```bash
just --justfile ops/ubelix/justfile sync-dashboard
```

Private Slurm env on UBELIX:

```bash
mkdir -p ~/.config/gammaboard
cat > ~/.config/gammaboard/slurm.env <<'EOF'
export SYMBOLICA_LICENSE="..."
export NO_SYMBOLICA_OEM_LICENSE=1
EOF
chmod 600 ~/.config/gammaboard/slurm.env
```

All UBELIX sbatch scripts source this file automatically and fail early if `SYMBOLICA_LICENSE` is missing.

## Build Images

Run these on a UBELIX login node.

Build GammaLoop:

```bash
mkdir -p logs/slurm/build logs/slurm/control logs/slurm/workers logs/postgres
sbatch ops/build/gammaloop.sbatch
```

Build GammaBoard:

```bash
mkdir -p logs/slurm/build logs/slurm/control logs/slurm/workers logs/postgres
sbatch ops/build/gammaboard.sbatch
```

Build behavior:
- images are commit-named
- `*-latest.sif` is updated as a symlink
- Rust build artifacts are reused from `/scratch/network/users/$USER/gammaboard-target/target`
- build logs go to `logs/slurm/build`

## Smoke Test

```bash
sbatch ops/slurm/smoke.sbatch
```

Optional overrides:

```bash
GAMMABOARD_WORKSPACE_ROOT=/absolute/path/to/itp_localunitaritydata
GAMMABOARD_IMAGE=/absolute/path/to/itp_localunitaritydata/images/gammaboard/gammaboard-latest.sif
GAMMABOARD_BIND_PATHS=/absolute/path/to/itp_localunitaritydata
```

## Single-Node Deploy

Run this on a UBELIX login node:

```bash
python ubelix.py up --single-node
```

This starts the same foreground deploy stack as the normal control job, but uses `config/server-single-node.toml`, where `allow_local_node_spawn = true`. Dashboard node-start actions spawn local worker processes inside the same Slurm allocation instead of creating external worker requests.

Direct Slurm submission also works when debugging:

```bash
sbatch ops/slurm/single_node_deploy.sbatch
```

## Control/UI Job

Run these on a UBELIX login node.

Prerequisite:

```bash
ls /storage/research/itp_localunitaritydata/artifacts/dashboard-build/index.html
```

Submit:

```bash
python ubelix.py up
```

`up` waits until Slurm assigned a node and nginx answers on the frontend port, then prints `frontend_ready=true`. While waiting, it ignores proxy environment variables for the readiness probe and periodically prints the latest control log tail.

To submit a new control job with a custom walltime:

```bash
python ubelix.py up --time 00:45:00
```

To also copy the printed tunnel command to your clipboard when the terminal supports OSC 52:

```bash
python ubelix.py up --copy
```

This job starts:
- local Postgres
- `gammaboard server`
- nginx serving the frontend and proxying `/api/*`

Then run the printed tunnel command on your local machine, for example:

```bash
ssh -N -L 8080:<control-node>:8080 <ubelix-user>@submit03.unibe.ch
```

The printed target defaults to `${USER}@submit03.unibe.ch` from the UBELIX login node. Override it when needed:

```bash
SSH_HOST=submit02.unibe.ch python ubelix.py up
SSH_TARGET=<user>@ubelix python ubelix.py up
```

Then open:

```text
http://localhost:8080
```

## Worker Model

Run these on a UBELIX login node.

Workers are simple:

```bash
gammaboard --runtime-config <runtime.toml> node run --name <node-name>
```

For multi-job mode:
- the control job owns Postgres and the API
- worker jobs connect using `GAMMABOARD_DATABASE_URL`
- the database must be reachable from other compute nodes
- workers launched through `ubelix.py` also watch the owning control Slurm job and stop when it disappears, even if Postgres is still reachable

Submit separate worker jobs:

```bash
python ubelix.py submit-workers --count 2 --prefix w
```

Resolve dashboard node-start requests automatically:

```bash
python ubelix.py watch-requests
```

The dashboard writes grouped startup requests into Postgres. `watch-requests` claims pending external requests, submits one Slurm worker job per requested node, and records submitted job ids back on the request row.

One-shot mode for debugging:

```bash
python ubelix.py watch-requests --once
```

If you run the multi-job control launcher in blocking mode, it also resolves requests while watching:

```bash
python ubelix.py up --watch
```

Stop a deployment:

```bash
python ubelix.py down
```

`down` requests `POST /api/nodes/stop-all`, waits for worker jobs to exit in multi-job mode, then uses `scancel` for remaining workers and finally the deploy job. It handles both `gb-ctl` and `gb-single`, but expects only one active deploy job.

If a control job exits unexpectedly, its sbatch cleanup also tries to stop the local Postgres daemon before stopping the Apptainer instance. Worker jobs submitted by `ubelix.py` receive the control job id and terminate themselves when that job is no longer active.

UBELIX sends `SIGCONT` followed by `SIGTERM` before `scancel` or time-limit cancellation and gives roughly 60 seconds before `SIGKILL`. Control and single-node deploy jobs let `gammaboard deploy run` receive that signal directly, request node shutdown, wait for sampler persistence, and stop local Postgres. Worker jobs use the same window to forward termination to their child node process.

## Config Model

Current UBELIX flow is intentionally simple:
- `WORKSPACE_ROOT` is the main override
- Slurm job behavior is mostly hardcoded in `slurm/*.sbatch`
- `ubelix.py` is the main operator CLI
- `ops/ubelix/justfile` is only for syncing and image pruning
- PostgreSQL logs go to `${WORKSPACE_ROOT}/logs/postgres`
- Slurm logs go to `${WORKSPACE_ROOT}/logs/slurm/{build,control,workers}`

## Design Choice

Use one control/UI job plus separate worker jobs.

Why:
- simple frontend access through one tunnel
- clean worker scaling via Slurm
- persistent DB data under workspace

The frontend node-start action creates DB-backed launch requests on UBELIX. `python ubelix.py watch-requests` resolves those requests into Slurm jobs; the control server does not spawn local child processes there.

## Notes

- `gammaboard` and `gammaloop` images are separate by design.
- The GammaBoard image packages only GammaBoard runtime artifacts.
- This is an operator guide for the current UBELIX setup, not a generic deployment system.
