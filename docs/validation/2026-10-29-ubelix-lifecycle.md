# UBELIX lifecycle consolidation — 2026-10-29

The control allocation now supervises the request launcher. Control and worker
commands share their pinned runtime configuration and container arguments. Healthy
workers use a deployment heartbeat; stale-control queries, including unknown
scheduler results, are cached across workers. A launcher pass batches scheduler
and accounting queries. Shutdown fences new submissions, preserves an accepted
in-flight receipt and stops subsequent submissions.

`down --force` supports an unavailable API while retaining Slurm ownership checks.
Worker allocations must disappear before control is cancelled. Unknown control
identity and cancellation timeouts leave cleanup incomplete and block database
reset. `reset-db` archives the stopped database and its deployment/configuration
metadata; `--discard` deletes the managed database paths. Both are scoped by
workspace and port offset and refuse symlink/external paths.

Slurm profiles replace duplicated batch-script resource defaults. Service builds
resolve their source revision and freeze their recipes before submission. Image
publication no longer changes selection; `select-image` is explicit. Pruning is a
dry run by default and preserves active/current deployments, rollback references,
database-archive image references and unknown/active builds. Private configuration,
physics states and process runtimes remain outside automatic pruning.

## Validation

All 41 helper tests passed with Python 3.13. They exercise:

- API-unreachable shutdown, ownership rejection, cancellation ordering and timeout;
- offset-scoped archive/discard, incompatible model preflight, modified snapshot
  rejection, path-escape rejection and overlapping active database protection;
- unchanged retained/current images, archive image references and pending build retention;
- concurrent frozen builds, a source branch advancing after submission, recipe tampering,
  smoke failure, and an older build finishing after a newer image was selected;
- pinned worker runtime arguments, offset handling, GPU flags, custom capabilities,
  memory overrides and worker deadline validation before submission intent;
- managed launcher supervision, in-flight receipt preservation, bounded process
  shutdown, shared heartbeat/accounting queries and scheduler-outage handling;
- existing launch-lease recovery, explicit submission repair and managed-tree sync.

Python static checks, shell syntax checks, CLI help, TOML parsing and whitespace
checks passed. Slurm and container execution are faked in these tests; actual
Apptainer image building, UBELIX scheduling policies and the scientific runtime
still require the deployment acceptance check in the
[UBELIX guide](../../ops/ubelix/README.md#deployment-acceptance-check).

Evidence is retained on the execution host under
`/common/dev/cedric/setup-logs/ubelix-lifecycle-20261029/`. No live UBELIX deployment
was changed during this work.
