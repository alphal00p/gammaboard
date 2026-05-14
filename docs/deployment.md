# Deployment

`gammaboard deploy run` is the normal dashboard stack supervisor. It starts the
backend API, nginx/frontend exposure, local or managed Postgres, and optional
local node runners according to the selected deploy/runtime/server configs.

## Profiles

- `local`: development profile for running from the checkout.
- `itphlies`: foreground-supervised deployment on the ITPhlies server.
- `ubelix`: Slurm-supervised control and worker jobs on UBELIX.

Profile-specific commands stay in `ops/*/README.md`. Shared config semantics are
documented in [config.md](config.md).

## Runtime Layout

Paths are resolved from the configured resources root unless explicitly
absolute. The default resource layout is:

```text
resources/
  runtimes/          Process runtime projects and packaged images
  states/            Shared model/integrator/checkpoint state
  templates/         Run, task, and node launch TOML templates
```

Deployment artifacts are separate from user resources:

```text
artifacts/           Build inputs, compiled binaries, package caches
db/                  Local Postgres state
images/              Deploy images such as gammaboard.sif and gammaloop.sif
logs/                Postgres, Slurm, and deployment logs
runtime/             Runtime sockets/pids/transient control files
```

## Image Model

UBELIX uses Apptainer images for deployable services:

- `gammaboard.sif`: Rust backend, nginx/Postgres dependencies, embedded
  frontend build, and process runtime launcher support.
- `gammaloop.sif`: GammaLoop execution image.
- Python or other process runtime images: task-specific evaluator/sampler
  environments under `resources/runtimes/...`.

Current image builds overwrite the latest image and write a small `.meta` file.
Older image generations are intentionally not retained.

## Nested Runtimes

Process tasks run explicit commands from their config. A command may directly
start a local binary, enter a Nix shell, or execute an Apptainer runtime image.
For nested Apptainer runtimes, the outer worker image launches the inner runtime
with normal bind paths. GPU-capable workers add NVIDIA passthrough with `--nv`.

The process protocol itself is packaging-agnostic. See
[process-runtime.md](process-runtime.md).

## Ports

Default ports are:

- Frontend: `8080`
- API: `4000`
- Postgres: `5400`

Deploy helpers support a port offset. Offset `1` shifts these to
`8081/4001/5401` and also suffixes local Postgres state paths where relevant.

## Access

Local and ITPhlies deployments usually expose the frontend directly on the
selected host and port. UBELIX deployments print an SSH tunnel command; open the
frontend through that tunnel from the workstation.
