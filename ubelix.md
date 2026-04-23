# UBELIX Deployment Notes

As of April 23, 2026, the UBELIX docs and admin guidance align with a containerized Slurm model for `gammaloop` + `gammaboard`. User services may listen on non-privileged ports inside temporary Slurm allocations, and browser/API access from outside the HPC network should use SSH tunnels.

## Practical Status

- Good fit: Slurm + Apptainer + job arrays map directly to the current runner model.
- Main blocker: database lifecycle/networking, not worker scheduling.
- Initial scripts now live in `ops/ubelix/`:
  - `slurm_smoke_container.sbatch`
  - `slurm_node_worker.sbatch`
  - `slurm_hello_control.sbatch`
  - `submit_hello.sh`

## Current UBELIX Constraints To Respect

- Services may listen inside Slurm allocations.
- Do not use privileged ports (`1-1023`).
- Ports are not exposed outside the HPC network except through SSH tunnels.
- PostgreSQL may run temporarily inside an allocation or as an external managed service.
- `apptainer` is available directly; no module load required.
- For account selection:
  - `gratis`: `--account=gratis` (debug-friendly QoS available).
  - `paygo`: also requires `--wckey=<project>`.
  - `teaching`: requires `--reservation=<reservation>`.
- For container bind mounts on UBELIX storage, bind full real paths (not only `/scratch`) because scratch/workspace paths are symlinked.
- For heavy Apptainer Docker conversion/build, use scratch-backed cache/temp directories (`APPTAINER_TMPDIR`, `APPTAINER_CACHEDIR`).

## Suggested Rollout

1. Build and validate the image with `ops/ubelix/slurm_smoke_container.sbatch`.
2. Use `ops/ubelix/submit_hello.sh` for first end-to-end hello-world with an external Postgres URL.
3. Add a self-contained allocation mode that starts temporary PostgreSQL, `gammaboard server`, sampler, and evaluators under one Slurm allocation.
4. Expose the dashboard with an SSH tunnel to the control job's compute node.
5. Move to external PostgreSQL only when run state must survive allocation expiry or multiple allocations need the same durable control plane.

## Sources

- https://hpc-unibe-ch.github.io/
- https://hpc-unibe-ch.github.io/runjobs/partitions/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/slurm-quickstart/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/container-jobs/
- https://hpc-unibe-ch.github.io/software/containers/apptainer/
- https://hpc-unibe-ch.github.io/storage/
- https://hpc-unibe-ch.github.io/storage/scratch/
