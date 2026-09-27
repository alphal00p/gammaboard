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
just sync-ubelix
```

The sync uploads `ops/`, shared `resources/templates/`, `docs/`, `ubelix.py`, and
this README. Only worker-allocation templates are specific to UBELIX. State,
checkpoints, databases, and runtime environments are not uploaded.
Pass a host and folder to sync into another relative path under `/storage/research/itp_localunitaritydata`:

```bash
just sync-ubelix ubelix "$USER/gammaboard"
```

## Build Images

Run on a UBELIX login node:

```bash
python ubelix.py build gammaloop --revision <COMMIT_OR_TAG>
python ubelix.py build gammaboard --revision <COMMIT_OR_TAG>
python ubelix.py build apptainer resources/runtimes/madnis/madnis.sif resources/runtimes/madnis/apptainer.def
```

GammaBoard and GammaLoop are service images under `images/`; each build resolves
the requested commit, tag, branch, or ref (default `HEAD`), records both the
request and resolved commit in
`images/<family>/<family>.meta`, and replaces the current image. Build logs go
to `logs/slurm/build`. Generic process-runtime builds take explicit output and
definition paths; stage the definition and its sources under
`resources/runtimes/` first.

For reproducible release images, pass a full 40-character commit SHA. The build
checks out that SHA detached and embeds it in the GammaBoard binary provenance.

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
`resources/templates/nodes`; cluster-specific node cards come from
`ops/ubelix/resources/templates/nodes` in the checkout. Run and task cards come
from the shared `resources/templates` tree. Supported `config` keys are `account`, `partition`, `qos`,
`wckey`, `reservation`, `gpu`, `gres`, `gpus`, `cpus_per_task`, `mem`,
`mem_per_cpu`, `time`, `constraint`, `nodelist`, and `exclude`.
Use `cores`, `nr_cores`, or `cpus` as dashboard-friendly aliases for `cpus_per_task`; they submit `--cpus-per-task=<value>` and register `cpus=<value>` as the worker capability.
Omitted group `config` defaults to `{}` and omitted `max_start_failures` defaults to `3`.
Workers with `gpu > 0` start the GammaBoard image with Apptainer `--nv`.
Nested process runtimes also include `--nv` in their explicit Apptainer command,
as shown in the shared MADNIS example.

`up --watch` also resolves dashboard requests while it watches the control job.

## Stop And Inspect

```bash
python ubelix.py status
python ubelix.py down
```

`down` requests node shutdown through the API, waits briefly for workers, cancels remaining worker jobs, then cancels the control or single-node job.

Use GammaBoard's `db backup` and `db restore` commands for database maintenance;
see [operations](docs/operations.md). The cluster helper manages Slurm jobs and
images; it does not delete database directories.

## Examples

Start with `resources/templates/runs/installation-smoke.toml`. After generating
the required state, use `resources/templates/runs/gammaloop.toml` for ttH
training/inference or `integration-campaign-qft-like.toml` for a campaign.
`resources/templates/runs/ghost_bump_madnis_apptainer.toml` uses the shared
MADNIS runtime image under `resources/runtimes/madnis/`. The same cards work on
local deployments with the corresponding dependencies. See [examples](docs/examples.md).

When the selected server config enables authentication, protected commands
require `--admin-password` or `GAMMABOARD_ADMIN_PASSWORD`. The checked-in
UBELIX configs are passwordless and should only be exposed to a trusted network.
PostgreSQL follows the same model: the control job enables
`--postgres-trusted-network`, whose `samenet trust` rule admits directly
connected UBELIX networks. Every user and device able to reach that port is
therefore part of the deployment trust boundary.

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
needed as an explicit override. GammaBoard and GammaLoop use their own bundled
Symbolica 3 application keys, so runtime Slurm jobs do not require
`SYMBOLICA_LICENSE`.
