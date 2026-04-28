# UBELIX Deployment Plan

This directory contains the current simplified UBELIX workflow with commit-named Apptainer images:
- a GammaLoop-only image builder,
- and a GammaBoard image builder that always processes both GammaLoop and GammaBoard targets.
- Native Rust compilation runs in Slurm jobs and reuses a persistent Cargo target cache on scratch storage.

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

- `config/submit_hello.env`: single source of truth for submit-time defaults (workspace/image/db/slurm/run names).
- `build/gammaloop.def`: builds a runtime image containing `gammaloop` only.
- `build/gammaboard.def`: builds a runtime image containing both `gammaloop` and `gammaboard`.
- `build/build_latest_gammaloop.sbatch`: always builds `gammaloop` from latest upstream `HEAD`, writes `gammaloop-<commit>.sif`, then updates `gammaloop-latest.sif` symlink.
- `build/build_latest_gammaboard.sbatch`: always builds both targets (`gammaloop`, then `gammaboard`) from latest upstream `HEAD`, writes commit-named images, and updates both latest symlinks.
- `config/runtime_external_db.template.toml`: worker/control runtime template for external DB mode (persistent data dir + scratch socket dir).
- `config/runtime_local_postgres.template.toml`: runtime template for future self-contained local-Postgres control jobs.
- `config/server.toml` and `config/deploy.toml`: UBELIX deploy/server profiles (same split as local/itphlies).
- `config/templates/runs` and `config/templates/tasks`: UBELIX-local run/task template directories used by `server.toml`.
- `slurm/smoke_container.sbatch`: no-DB smoke check (`gammaloop`/`gammaboard` version/help) for a runtime image.
- `slurm/node_worker.sbatch`: long-running worker (`node run`) for sampler/evaluator.
- `slurm/hello_control.sbatch`: creates a tiny run, appends one sample task, auto-assigns workers, waits for completion.
- `justfile` recipe `submit-hello`: submits one sampler job + evaluator array + control job with dependencies.
- `justfile` recipe `submit-hello-single`: submits one all-in-one hello job (local Postgres + workers + control) for strict QOS submit limits.

## Config Model

Use `config/submit_hello.env` as the primary config source.

- Put persistent defaults there (`WORKSPACE_ROOT`, `IMAGE_PATH`, `DATABASE_URL`, `ACCOUNT`, `PARTITION`, `QOS`, times, run names).
- Keep `justfile` recipes thin: source env file, run preflight checks, submit jobs.
- Allow one-off overrides via shell env vars when needed.
- Logs are submitted with absolute `--output/--error` paths under `${WORKSPACE_ROOT}/logs/...` from `just` submit commands.

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

## 1) Build Commit-Named Runtime Images

Why:

- `gammaloop` needs system build dependencies that are not guaranteed to exist on UBELIX by default.
- Commit-named image files keep artifact history readable.
- `*-latest.sif` symlinks make operator-facing paths stable.
- Rust build artifacts persist in `/scratch/network/users/$USER/gammaboard-target/target` across build jobs.

Definition files:

- [build/gammaloop.def](/home/cedricsigrist/Workspace/repos/gammaboard/ops/ubelix/build/gammaloop.def)
- [build/gammaboard.def](/home/cedricsigrist/Workspace/repos/gammaboard/ops/ubelix/build/gammaboard.def)

Build GammaLoop image only:

```bash
mkdir -p logs/build logs/control logs/workers
sbatch ops/ubelix/build/build_latest_gammaloop.sbatch
```

Build GammaBoard image (this always processes both GammaLoop and GammaBoard targets):

```bash
mkdir -p logs/build logs/control logs/workers
sbatch ops/ubelix/build/build_latest_gammaboard.sbatch
```

Both build scripts compile Rust natively on compute nodes first, using:

```bash
CARGO_TARGET_DIR=/scratch/network/users/$USER/gammaboard-target/target
```

Then they build SIF images from package-only `.def` files that install binaries from:

```text
<WORKSPACE_ROOT>/artifacts/bin/gammaloop
<WORKSPACE_ROOT>/artifacts/bin/gammaboard
```

Edit the config block at the top of these sbatch files directly on UBELIX if you want to change workspace paths, repositories, or scratch/cache roots.

By default, each build job:
- resolves upstream repository `HEAD` commit SHA(s),
- always builds new images named with those commit SHA(s),
- updates `images/<kind>/<kind>-latest.sif` as a symlink to the commit-named image.
- writes a sibling `.meta` file with provenance and coarse timing fields.

Each `.meta` now includes: repo SHA(s), dirty/clean source status, Slurm job id, build host, toolchain versions, cargo profile/features, target dir, image size/checksum, and phase timings (`timing_fetch_checkout_seconds`, `timing_cargo_gammaloop_seconds`, optional `timing_cargo_gammaboard_seconds`, `timing_binary_stage_seconds`, `timing_apptainer_gammaloop_seconds`, optional `timing_apptainer_gammaboard_seconds`, `timing_total_seconds`).

If you want to debug the build interactively first, use an interactive Slurm allocation and then run the same `apptainer build ...` command there. Avoid doing the full build on the login node.

UBELIX specifically recommends scratch-backed cache dirs when pulling or building from Docker containers:

```bash
mkdir -p /scratch/network/users/$USER
export APPTAINER_TMPDIR=/scratch/network/users/$USER/apptainer/tmp
export APPTAINER_CACHEDIR=/scratch/network/users/$USER/apptainer/cache
apptainer build --notest gammaloop.sif ops/ubelix/build/gammaloop.def
apptainer build --notest gammaboard.sif ops/ubelix/build/gammaboard.def
```

Output layout:

```text
<WORKSPACE_ROOT>/
  images/gammaloop/
    gammaloop-<commit>.sif
    gammaloop-latest.sif -> gammaloop-<commit>.sif
  images/gammaboard/
    gammaboard-<commit>.sif
    gammaboard-latest.sif -> gammaboard-<commit>.sif
```

The build jobs keep persistent source checkouts under `artifacts/src/*`, staged binaries under `artifacts/bin/*`, and a persistent compile cache under scratch (`RUST_TARGET_BASE`) to accelerate incremental rebuilds.

## 2) Runtime Image And Smoke Test

Once you have such a runtime image, smoke test it with:

```bash
sbatch ops/ubelix/slurm/smoke_container.sbatch
```

Optional environment overrides:

```bash
GAMMABOARD_WORKSPACE_ROOT=/absolute/path/to/itp_localunitaritydata
GAMMABOARD_IMAGE=/absolute/path/to/itp_localunitaritydata/images/gammaboard/gammaboard-latest.sif
GAMMABOARD_BIND_PATHS=/absolute/path/to/itp_localunitaritydata
```

## 3) End-To-End Hello Test (External Postgres Path)

```bash
export GAMMABOARD_WORKSPACE_ROOT=/absolute/path/to/itp_localunitaritydata
export GAMMABOARD_IMAGE=/absolute/path/to/itp_localunitaritydata/images/gammaboard/gammaboard-latest.sif
export GAMMABOARD_DATABASE_URL=postgresql://postgres:postgres@127.0.0.1:5433/gammaboard_db
export DEPLOY_NAME=default
just --justfile ops/ubelix/justfile submit-hello
```

Useful overrides:

```bash
ACCOUNT=gratis
PARTITION=epyc2
QOS=job_debug
EVALUATOR_COUNT=2
RUN_NAME=ubelix-hello
NODE_PREFIX=gb-hello
DEPLOY_NAME=default
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
<workspace>/itp_localunitaritydata/
  images/
    gammaboard/
      gammaboard-<commit>.sif
      gammaboard-latest.sif -> gammaboard-<commit>.sif
    gammaloop/
      gammaloop-<commit>.sif
      gammaloop-latest.sif -> gammaloop-<commit>.sif
  ops/
    ubelix/
      build/
        gammaloop.def
        gammaboard.def
        build_latest_gammaloop.sbatch
        build_latest_gammaboard.sbatch
      slurm/
        smoke_container.sbatch
        node_worker.sbatch
        hello_control.sbatch
      justfile
  runtime/
    <deploy-name>.toml
  db/
    <deploy-name>/
      data/
      socket/
      logfile
  states/
    gammaloop/
      <state-name>/
  artifacts/
    <run-id-or-name>/
  logs/
    build/
    control/
    workers/
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
- The worker/control scripts render runtime TOML templates via `envsubst` (GNU `gettext` package).
- This is an initial smoke/hello setup, not a production deployment profile.
