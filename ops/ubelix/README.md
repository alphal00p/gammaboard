# UBELIX Deployment Plan

This directory contains first-pass Slurm jobs for testing `gammaloop` and `gammaboard` on UBELIX with an Apptainer image built from [`gammaboard.def`](/home/cedricsigrist/Workspace/repos/gammaboard/gammaboard.def).

## Deployment Decision

Use a self-contained Slurm allocation first:

- Run PostgreSQL inside the allocation on a non-privileged port.
- Run `gammaboard server` inside the same allocation on a non-privileged port.
- Run one sampler-aggregator and an evaluator array as Slurm jobs using the same database URL.
- Serve the dashboard from the same control job initially.
- Access the dashboard/API through one SSH tunnel from the compute node to your local machine.

This matches UBELIX policy: user services may listen inside temporary allocations, privileged ports `1-1023` are not allowed, and access from outside the HPC network must go through SSH tunneling. External managed PostgreSQL remains a valid later option, but it is not necessary for initial deployment.

The frontend can be separated later, but the first deployment should serve frontend and API from the same forwarded origin. That avoids CORS, avoids exposing a second tunnel, and keeps the operator workflow to one URL.

## Files

- `slurm_smoke_container.sbatch`: no-DB smoke check (`gammaloop`/`gammaboard` version/help).
- `slurm_node_worker.sbatch`: long-running worker (`node run`) for sampler/evaluator.
- `slurm_hello_control.sbatch`: creates a tiny run, appends one sample task, auto-assigns workers, waits for completion.
- `submit_hello.sh`: submits one sampler job + evaluator array + control job with dependencies.

## Target Topology

Initial single-allocation deployment:

```text
local browser
  |
  | ssh -L 8080:<compute-node>:8080
  v
UBELIX login node
  |
  v
compute allocation
  - control job
    - PostgreSQL on <compute-node>:<db-port>
    - gammaboard API on 127.0.0.1:4000 or 0.0.0.0:4000
    - static frontend / reverse proxy on 0.0.0.0:8080
  - worker jobs
    - sampler-aggregator node
    - evaluator nodes
```

This is the simplest robust path for first production-like testing because database, API, and workers share allocation lifetime. When the allocation ends, the deployment ends cleanly. Persist database data on workspace/scratch if checkpoint state should survive a job timeout; use local node scratch only for disposable smoke runs.

External-DB deployment:

```text
external PostgreSQL
  ^
  |
UBELIX worker jobs + optional dashboard/API job
```

Use this once we need run state to survive allocation expiry without copying a local Postgres data directory, or when multiple independent allocations should attach to the same control plane.

## 1) Build Image On UBELIX

If Docker-conversion build cache is too heavy for `/tmp`, UBELIX recommends scratch-backed cache dirs:

```bash
mkdir -p /scratch/network/users/$USER
export APPTAINER_TMPDIR=/scratch/network/users/$USER
export APPTAINER_CACHEDIR=/scratch/network/users/$USER
apptainer build --notest gammaboard.sif gammaboard.def
```

## 2) Smoke Test (No Database Needed)

```bash
sbatch ops/ubelix/slurm_smoke_container.sbatch
```

Optional environment overrides:

```bash
GAMMABOARD_IMAGE=/absolute/path/to/gammaboard.sif
GAMMABOARD_BIND_PATHS=/absolute/path/to/workspace
```

## 3) End-To-End Hello Test (External Postgres Path)

```bash
export GAMMABOARD_IMAGE=/absolute/path/to/gammaboard.sif
export GAMMABOARD_PROJECT_ROOT=/absolute/path/to/gammaboard
export GAMMABOARD_DATABASE_URL=postgresql://<user>:<pass>@<host>:5432/<db>
./ops/ubelix/submit_hello.sh
```

Useful overrides:

```bash
ACCOUNT=gratis
PARTITION=epyc2
QOS=job_debug
EVALUATOR_COUNT=2
RUN_NAME=ubelix-hello
NODE_PREFIX=gb-hello
EXTRA_SBATCH_ARGS="--wckey=<project>"
```

For `teaching`, use e.g. `EXTRA_SBATCH_ARGS="--reservation=<reservation>"`.

## 4) Planned Self-Contained Allocation Flow

The next scripts should automate this sequence:

1. Allocate one control job that creates or reuses a run-specific Postgres data directory under workspace/scratch.
2. Start PostgreSQL with `gammaboard db start` using a runtime config whose `local_postgres` paths point at that persistent directory.
3. Start `gammaboard server` in that same control job.
4. Serve the built frontend from the same job, preferably behind one reverse proxy on `0.0.0.0:8080` forwarding `/api/` to the API.
5. Print the compute node name and the SSH tunnel command:
   ```bash
   ssh -N -L 8080:<compute-node>:8080 <user>@submit03.unibe.ch
   ```
6. Submit or launch worker jobs with `GAMMABOARD_DATABASE_URL` set to the control job's Postgres URL.
7. Run `gammaboard run add ...`, then `gammaboard auto-assign <RUN> <EVALUATOR_COUNT>`.
8. On teardown, pause runs, stop nodes, stop the server/frontend proxy, and stop Postgres.

For resumable runs, the control job should not delete the Postgres data directory on normal shutdown. It should let Postgres flush to disk and exit cleanly. Deleting or archiving the database should be a separate explicit operation.

## Database Lifecycle

Recommended layout:

```text
<workspace>/gammaboard-ubelix/
  images/gammaboard.sif
  db/<deploy-name>/data
  db/<deploy-name>/socket
  db/<deploy-name>/logfile
  runtime/<deploy-name>.toml
  logs/
```

The control job should:

1. Create the runtime config from parameters (`deploy-name`, `db-port`, `api-port`, storage root).
2. Run `gammaboard db start`, which initializes the database if needed, starts Postgres, applies migrations, and enables `pg_stat_statements`.
3. Trap `SIGTERM`, `SIGINT`, and shell `EXIT`.
4. On shutdown, request node shutdown / pause active runs, then run `gammaboard db stop`.
5. Leave the data directory intact for resume.

Use local node scratch only for disposable tests. Use workspace-backed storage for anything intended to resume after walltime expiry.

## Frontend Recommendation

Serve the frontend from the same control job for the first deployment.

Why:

- One SSH tunnel and one browser origin (`http://localhost:8080`).
- No CORS/origin mismatch between dashboard and API.
- The frontend is static; serving it from the control job is low overhead compared to the workers.
- The existing `gammaboard deploy` model already supports a reverse-proxy style deployment where static files and `/api/` share one public origin.

Separate the frontend later only if local development convenience or a managed web host matters. If separated, the API server config must explicitly allow the frontend origin, and operators may need two tunnels or a local frontend build that calls the forwarded API.

## Worker Model

Worker jobs stay simple and identical:

```bash
gammaboard --runtime-config <runtime.toml> node run --name <node-name>
```

Workers should receive the control job's `GAMMABOARD_DATABASE_URL`, announce themselves, and wait for assignment. The control job or an operator can then run:

```bash
gammaboard --runtime-config <runtime.toml> auto-assign <RUN> <EVALUATOR_COUNT>
```

The sampler-aggregator remains at most one assigned node per run. Evaluators can be a Slurm array.

## Open Design Point

There are two viable ways to launch workers:

- Separate Slurm jobs/arrays: better scheduler visibility and flexible scaling; requires Postgres to listen on a hostname/interface reachable from other compute jobs.
- Child processes inside one Slurm allocation: simpler networking (`127.0.0.1` works) and simpler teardown; less flexible if we want to resize evaluators independently.

For UBELIX production-like runs, prefer separate worker jobs once the DB listener binding is confirmed. For first self-contained testing, child processes inside one allocation may be faster to validate.

Important implementation detail: if workers are separate Slurm jobs, PostgreSQL must listen on a compute-node hostname/interface reachable inside the HPC network, not only on a private Unix socket. If all roles run as processes inside one allocation, `127.0.0.1` is sufficient.

## Notes

- `GAMMABOARD_DATABASE_URL` must point to a Postgres service reachable from all participating jobs.
- The target database must already have the `gammaboard` schema/migrations applied.
- The scripts use `apptainer exec -B <project-root> ...`; bind full storage paths on UBELIX.
- This is an initial smoke/hello setup, not a production deployment profile.
