# UBELIX Deployment

Operator guide for the current UBELIX setup. Local sync commands run from your workstation; `ubelix.py` commands run on a UBELIX login node.

## Model

- Workspace: `/storage/research/itp_localunitaritydata/gammaboard`
- Images: separate Apptainer images for `gammaboard` and `gammaloop`
- Deploy: one control/UI Slurm job with Postgres, API, nginx, and frontend
- Workers: separate Slurm jobs connecting to the control job database
- Access: one SSH tunnel to the frontend port
- Resources: relative run/task paths resolve under `${WORKSPACE_ROOT}/resources`; GammaLoop states use `states/...`
- Nix: the GammaBoard image includes the Nix CLI and binds `${WORKSPACE_ROOT}/nix` as `/nix`

## Sync From Local

```bash
just --justfile ops/ubelix/justfile sync-ops
```

`sync-ops` uploads `ops/`, `ubelix.py`, and this README. The justfile stays local.

## Build Images

Run on a UBELIX login node:

```bash
python ubelix.py build gammaloop
python ubelix.py build gammaboard
```

The GammaBoard build also builds and embeds the dashboard frontend. Each build overwrites `images/<family>/<family>.sif` and writes `images/<family>/<family>.meta`. Build logs go to `logs/slurm/build`.

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

`up --watch` also resolves dashboard requests while it watches the control job.

## Stop And Inspect

```bash
python ubelix.py status
python ubelix.py down
```

`down` requests node shutdown through the API, waits briefly for workers, cancels remaining worker jobs, then cancels the control or single-node job.

Admin-protected commands accept `--admin-password` or `GAMMABOARD_ADMIN_PASSWORD`.

## Layout

```text
<WORKSPACE_ROOT>/
  ops/{build,config,slurm}/
  ubelix.py
  README.md
  artifacts/{bin,npm-cache,sqlx-root,src}/
  db/<deploy-name>/
  images/{gammaboard,gammaloop}/
  logs/{postgres,slurm}/
  nix/
  resources/states/
  runtime/
```

Secrets and local overrides live in `${HOME}/.config/gammaboard/slurm.env`; all sbatch scripts source it and require `SYMBOLICA_LICENSE`.
