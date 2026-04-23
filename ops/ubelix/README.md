# UBELIX Hello-World Jobs

This directory contains first-pass Slurm jobs for testing `gammaloop` and `gammaboard` on UBELIX with an Apptainer image built from [`gammaboard.def`](/home/cedricsigrist/Workspace/repos/gammaboard/gammaboard.def).

## Files

- `slurm_smoke_container.sbatch`: no-DB smoke check (`gammaloop`/`gammaboard` version/help).
- `slurm_node_worker.sbatch`: long-running worker (`node run`) for sampler/evaluator.
- `slurm_hello_control.sbatch`: creates a tiny run, appends one sample task, auto-assigns workers, waits for completion.
- `submit_hello.sh`: submits one sampler job + evaluator array + control job with dependencies.

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

## 3) End-To-End Hello Test (Requires Shared Postgres)

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

## Notes

- `GAMMABOARD_DATABASE_URL` must point to a Postgres service reachable from compute nodes.
- The target database must already have the `gammaboard` schema/migrations applied.
- The scripts use `apptainer exec -B <project-root> ...`; bind full storage paths on UBELIX.
- This is an initial smoke/hello setup, not a production deployment profile.
