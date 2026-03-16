# UBELIX Deployment Notes

## Summary

Deploying Gammaboard on UBELIX looks feasible, but only if PostgreSQL is available as a stable service reachable from all worker jobs. Slurm, job arrays, interactive jobs, and Apptainer fit the current runtime model well. The main operational friction is the database dependency, not compute scheduling.

## Good Fit

- UBELIX supports Slurm batch jobs, interactive jobs, job arrays, and Apptainer containers.
- Gammaboard already maps well to one sampler-aggregator plus many evaluator workers.
- Evaluators can scale out naturally as separate Slurm jobs or array tasks.

## Recommended Deployment Shape

- Package `gammaboard` as a binary or Apptainer image.
- Run one sampler-aggregator as a small Slurm job.
- Run evaluators as a Slurm job array or multiple tasks in one allocation.
- Keep PostgreSQL outside worker jobs as a stable shared service.
- Run the dashboard/backend outside UBELIX compute jobs, or in a separate interactive/web-facing session.

## Main Risks

- PostgreSQL is required as the live control plane and work queue.
- Scaling will likely become DB-bound before it becomes UBELIX-bound.
- `run-node` currently handles `ctrl_c`, but Slurm cancellation uses `SIGTERM`; graceful checkpoint/snapshot handling should be improved before production use.
- Running the dashboard on-cluster is possible in principle, but the simplest setup is to host it elsewhere.

## Ease Of Use

Good once wrapped for Slurm, not turnkey today.

Best next steps:
- add `SIGTERM` handling for clean sampler snapshot persistence
- define `sbatch` wrappers for sampler and evaluator arrays
- decide on external Postgres hosting
- reduce DB pressure before scaling far

## MPI Work Queue

Moving the work queue to MPI would improve raw intra-allocation throughput, but it is probably the wrong primary direction for this project.

### Pros

- much lower queue latency than PostgreSQL
- less DB polling and write amplification
- good fit for tightly coupled workers inside one allocation

### Cons

- much worse operational ergonomics than the current DB-backed control plane
- harder recovery, pause/resume, and observability across job restarts
- MPI works best inside one allocation, while Gammaboard currently supports loose, durable coordination across processes and nodes
- sampler, evaluators, dashboard, and CLI steering would all become harder to decouple
- it would likely replace a durable queue with an ephemeral one, which is a large architectural regression for this system

## Recommendation On MPI

Do not replace the main work queue with MPI.

If needed, use MPI only as an optional fast path inside evaluator-side execution within a single Slurm allocation. Keep PostgreSQL as the durable control plane, assignment store, snapshot store, and source of truth.

## Sources

- https://hpc-unibe-ch.github.io/
- https://hpc-unibe-ch.github.io/firststeps/accessUBELIX/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/submission/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/interactive/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/throughput/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/container-jobs/
- https://hpc-unibe-ch.github.io/software/containers/apptainer/
- https://hpc-unibe-ch.github.io/firststeps/loggingin-webui/
- https://hpc-unibe-ch.github.io/runjobs/scheduled-jobs/checkpointing/
