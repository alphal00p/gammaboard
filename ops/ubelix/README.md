# UBELIX Deployment

Current model:
- build `gammaloop` and `gammaboard` as separate Apptainer images
- run control/UI as one Slurm job
- run workers as separate Slurm jobs against the control job database
- access the frontend through one SSH tunnel

Execution context:
- run `python ops/ubelix.py up` on an UBELIX login node
- `python ops/ubelix.py up` prints the SSH tunnel command to run on your local machine
- run `just --justfile ops/ubelix/justfile sync-ops` and `sync-dashboard` on your local machine

## Layout

Repo source lives under `ops/ubelix/` and mirrors this server-side layout directly:

```text
<WORKSPACE_ROOT>/
  ops/
    ubelix.py
    build/
    config/
    slurm/
  justfile
  README-ubelix.md
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

- `ops/build/gammaloop.sbatch`
- `ops/build/gammaboard.sbatch`
- `ops/slurm/smoke.sbatch`
- `ops/slurm/hello.sbatch`
- `ops/slurm/control.sbatch`
- `ops/slurm/worker.sbatch`
- `ops/ubelix.py`
- `ops/config/runtime/local_postgres.template.toml`
- `ops/config/runtime/external_db_control.template.toml`
- `ops/config/runtime/external_db_worker.template.toml`
- `ops/config/server/server.toml`
- `ops/config/deploy/deploy.toml`

Notes:
- UBELIX runtime templates set `resources.roots = ["${WORKSPACE_ROOT}/states"]`.
- The UBELIX server profile sets `allow_local_node_spawn = false`.
- Worker runtime TOML is rendered per job via `envsubst`.

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

## Hello Test

Run this on a UBELIX login node:

```bash
mkdir -p logs/slurm/control logs/slurm/workers logs/postgres
sbatch ops/slurm/hello.sbatch
```

## Control/UI Job

Run these on a UBELIX login node.

Prerequisite:

```bash
ls /storage/research/itp_localunitaritydata/artifacts/dashboard-build/index.html
```

Submit:

```bash
python ops/ubelix.py up
```

`up` waits until Slurm assigned a node and nginx answers on the frontend port, then prints `frontend_ready=true`. To only submit/reuse the job and print the tunnel hint immediately:

```bash
python ops/ubelix.py up --no-wait
```

To also copy the printed tunnel command to your clipboard when the terminal supports OSC 52:

```bash
python ops/ubelix.py up --copy
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
SSH_HOST=submit02.unibe.ch python ops/ubelix.py up
SSH_TARGET=<user>@Ubelix python ops/ubelix.py up
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

Submit separate worker jobs:

```bash
python ops/ubelix.py submit-workers --count 2 --prefix w
```

Resolve dashboard node-start requests automatically:

```bash
python ops/ubelix.py watch-requests
```

The dashboard writes grouped startup requests into Postgres. `watch-requests` claims pending external requests, submits one Slurm worker job per requested node, and records submitted job ids back on the request row.

One-shot mode for debugging:

```bash
python ops/ubelix.py watch-requests --once
```

If you run the control launcher in blocking mode, it also resolves requests while watching:

```bash
python ops/ubelix.py up --watch
```

Stop a deployment:

```bash
python ops/ubelix.py down
```

`down` requests `POST /api/nodes/stop-all`, waits for worker jobs to exit, then uses `scancel` for remaining workers and finally the control job.

## Config Model

Current UBELIX flow is intentionally simple:
- `WORKSPACE_ROOT` is the main override
- Slurm job behavior is mostly hardcoded in `ops/slurm/*.sbatch`
- `ops/ubelix.py` is the main operator CLI
- `ops/ubelix/justfile` is only for syncing and image pruning
- PostgreSQL logs go to `${WORKSPACE_ROOT}/logs/postgres`
- Slurm logs go to `${WORKSPACE_ROOT}/logs/slurm/{build,control,workers}`

## Design Choice

Use one control/UI job plus separate worker jobs.

Why:
- simple frontend access through one tunnel
- clean worker scaling via Slurm
- persistent DB data under workspace

The frontend node-start action creates DB-backed launch requests on UBELIX. `python ops/ubelix.py watch-requests` resolves those requests into Slurm jobs; the control server does not spawn local child processes there.

## Notes

- `gammaboard` and `gammaloop` images are separate by design.
- The GammaBoard image packages only GammaBoard runtime artifacts.
- This is an operator guide for the current UBELIX setup, not a generic deployment system.
