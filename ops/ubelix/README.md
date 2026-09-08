# UBELIX Deployment

Operator guide for the current UBELIX setup. Local sync commands run from your workstation; `ubelix.py` commands run on a UBELIX login node.

Shared docs live in repo-root `docs/`: `docs/deployment.md`, `docs/config.md`, and `docs/operations.md`.

## UBELIX Model

- Workspace: the directory containing the installed `ubelix.py`
- Deploy: one control/UI Slurm job with Postgres, API, nginx, and frontend
- Workers: separate Slurm jobs connecting to the control job database
- Access: one SSH tunnel to the frontend port
- Resources: templates, states, and process runtimes live under
  `${WORKSPACE_ROOT}/resources`

## Sync From Local

```bash
just --justfile ops/ubelix/justfile sync-ops
```

`sync-ops` uploads `ops/`, `docs/`, `ubelix.py`, and this README. The justfile stays local.
Pass `remote_folder` to sync into a different relative path under `/storage/research/itp_localunitaritydata`:

```bash
just --justfile ops/ubelix/justfile remote_folder=$USER/gammaboard sync-ops
```

## Build Images

Run on a UBELIX login node:

```bash
python ubelix.py build gammaloop
python ubelix.py build gammaboard
python ubelix.py build apptainer resources/runtimes/madnis/madnis.sif resources/runtimes/madnis/apptainer.def
```

GammaBoard and GammaLoop are service images under `images/`; each build resolves
the remote repository's current `HEAD`, records the chosen commit in
`images/<family>/<family>.meta`, and replaces the current image. Build logs go
to `logs/slurm/build`. Generic process-runtime builds take explicit output and
definition paths; stage the definition and its sources under
`resources/runtimes/` first.

## Start

Normal multi-job deployment:

```bash
python ubelix.py up
```

Single-node deployment with local workers in the same Slurm allocation:

```bash
python ubelix.py up --single-node
```

Useful options:

```bash
python ubelix.py up --time 00:45:00
python ubelix.py up --port-offset 10
python ubelix.py up --watch
python ubelix.py up --copy
```

`up` waits for Slurm node assignment and frontend readiness, then prints the SSH tunnel command. Run that command locally and open `http://localhost:8080`.

Port offsets shift frontend/API/Postgres from `8080/4000/5400`; pass the same `--port-offset` to helper commands for that deployment.

## Workers

Submit manual workers:

```bash
python ubelix.py submit-workers --count 2 --prefix w
```

Resolve dashboard node-start requests:

```bash
python ubelix.py watch-requests
python ubelix.py watch-requests --once
```

Dashboard node-launch TOML uses grouped `config` tables. On UBELIX, the watcher maps supported config keys to `sbatch` options. A GPU group such as:

```toml
[[groups]]
count = 1
name_prefix = "gpu"
max_start_failures = 6
config = { gpu = "rtx4090:1" }
```

submits worker jobs with `--gres=gpu:rtx4090:1 --partition=gpu` and registers
`gpu=1` as a worker capability. Select another free-tier GPU type with
`config = { gpu = "h100:1" }` when available. Synced templates live under
`resources/templates`; their source is `ops/ubelix/resources/templates` in the
local checkout. Supported `config` keys are `account`, `partition`, `qos`,
`wckey`, `reservation`, `gpu`, `gres`, `gpus`, `cpus_per_task`, `mem`,
`mem_per_cpu`, `time`, `constraint`, `nodelist`, and `exclude`.
Use `cores`, `nr_cores`, or `cpus` as dashboard-friendly aliases for `cpus_per_task`; they submit `--cpus-per-task=<value>` and register `cpus=<value>` as the worker capability.
Omitted group `config` defaults to `{}` and omitted `max_start_failures` defaults to `3`.
Workers with `gpu > 0` start the GammaBoard image with Apptainer `--nv`, so nested Python Apptainer runtimes can request NVIDIA passthrough with `nv = true`.

`up --watch` also resolves dashboard requests while it watches the control job.

## Stop And Inspect

```bash
python ubelix.py status
python ubelix.py down
```

`down` requests node shutdown through the API, waits briefly for workers, cancels remaining worker jobs, then cancels the control or single-node job.

When the selected server config enables authentication, protected commands
require `--admin-password` or `GAMMABOARD_ADMIN_PASSWORD`. The checked-in
UBELIX configs are passwordless and should only be exposed to a trusted network.

## UBELIX Layout

```text
<WORKSPACE_ROOT>/
  ops/{build,config,slurm}/
  ubelix.py
  README.md
  artifacts/{bin,npm-cache,sqlx-root,src}/
  images/{gammaboard,gammaloop}/      # service images
  logs/slurm/
  resources/db/{postgres,socket,logfile}
  resources/runtimes/                 # process evaluator/sampler runtimes
  resources/states/
```

Local overrides live in `${HOME}/.config/gammaboard/slurm.env`; all sbatch
scripts source it when present. The workspace is self-locating from the
installed `ubelix.py` and sbatch paths, so `GAMMABOARD_WORKSPACE_ROOT` is only
needed as an explicit override. GammaBoard/GammaLoop use the Symbolica OEM
license compiled during the build jobs, so runtime Slurm jobs do not require
`SYMBOLICA_LICENSE`.
