# UBELIX Deployment Notes

As of April 23, 2026, the UBELIX docs align with a containerized Slurm model for `gammaloop` + `gammaboard`, with one key constraint: keep PostgreSQL as an external shared service reachable by all jobs.

## Practical Status

- Good fit: Slurm + Apptainer + job arrays map directly to the current runner model.
- Main blocker: database service lifecycle/networking, not worker scheduling.
- Initial scripts now live in `ops/ubelix/`:
  - `slurm_smoke_container.sbatch`
  - `slurm_node_worker.sbatch`
  - `slurm_hello_control.sbatch`
  - `submit_hello.sh`

## Current UBELIX Constraints To Respect

- `apptainer` is available directly; no module load required.
- For account selection:
  - `gratis`: `--account=gratis` (debug-friendly QoS available).
  - `paygo`: also requires `--wckey=<project>`.
  - `teaching`: requires `--reservation=<reservation>`.
- For container bind mounts on UBELIX storage, bind full real paths (not only `/scratch`) because scratch/workspace paths are symlinked.
- For heavy Apptainer Docker conversion/build, use scratch-backed cache/temp directories (`APPTAINER_TMPDIR`, `APPTAINER_CACHEDIR`).

## Suggested Rollout

1. Build and validate the image with `ops/ubelix/slurm_smoke_container.sbatch`.
2. Point jobs to a stable Postgres URL (`GAMMABOARD_DATABASE_URL`).
3. Use `ops/ubelix/submit_hello.sh` for first end-to-end hello-world (1 sampler + evaluator array + control job).
4. Split control-plane and workers for production:
   - persistent backend/control process,
   - separate long-lived sampler/evaluator worker jobs.

## Sources

- https://hpc-unibe-ch.github.io/
- https://hpc-unibe-ch.github.io/runjobs/partitions/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/slurm-quickstart/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/container-jobs/
- https://hpc-unibe-ch.github.io/software/containers/apptainer/
- https://hpc-unibe-ch.github.io/storage/
- https://hpc-unibe-ch.github.io/storage/scratch/
