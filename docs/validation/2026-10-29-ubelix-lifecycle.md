# UBELIX lifecycle and checkout deployment — 2026-10-29

The control allocation supervises the request launcher. Control and workers share
configuration snapshots and container arguments. Healthy workers use a deployment
heartbeat; stale-control scheduler queries are cached across workers. A launcher
pass batches scheduler/accounting queries. Shutdown fences submissions, preserves
an accepted in-flight receipt and stops subsequent launches.

`down --force` supports an unavailable API while retaining job ownership checks.
Workers disappear before control is cancelled. Unknown control identity or a
cancellation timeout blocks database reset. `reset-db` archives data/configuration;
`--discard` deletes them. Both are scoped by workspace/offset and reject external
or symlinked database paths.

The deployment interface was then simplified to a Git checkout and one ignored
`ops/ubelix/local.toml`, containing the shared workspace path and overrides.
Managed-tree sync, image selection/history, revision flags, separate override
files, duplicate single-node defaults and artifact-pruning policy were removed.
Builds snapshot the checked-out commit and job scripts before submission. Each
successful service build replaces a fixed image; compilation/smoke failures retain
the previous image. Workspace builds and deployments exclude one another. Rollback
requires checking out an earlier commit and rebuilding. Database archives retain
provenance/configuration but no image history.

Templates are seeded into the workspace once and remain editable without dirtying
the checkout. Later starts preserve edits and deletions. Successful service builds
remove source/temp staging; logs, records and failed build evidence remain available.

## Validation

All 47 helper tests passed with Python 3.13. They exercise:

- real Git checkout snapshots, dirty-source rejection, fixed-image replacement,
  failed smoke preservation and queued builds surviving checkout removal;
- build/deployment exclusion across offsets, queued/running build exclusion,
  scheduler failure, local GammaLoop sources and frozen custom runtime definitions;
- local settings independent of working directory or workspace environment variables,
  memory-profile overrides, and preservation of edited/deleted dashboard templates;
- API-unreachable shutdown, ownership rejection, cancellation ordering and timeout;
- offset-scoped archive/discard, incompatible-model preflight, modified-snapshot
  rejection, path-escape rejection and overlapping active database protection;
- pinned worker runtime arguments, port-offset handling, GPU flags, capabilities,
  memory overrides and worker deadline validation before submission intent;
- launcher supervision, in-flight receipt preservation, bounded process shutdown,
  shared heartbeat/accounting queries and scheduler-outage handling;
- launch-lease recovery and explicit submission repair.
- actionable startup errors for unsupported Python versions and rejection of
  tagged digest references before invoking Apptainer.
- execution of each `%post` stage's `%setup` hook against an isolated root
  (including paths with spaces), checking that `/cache` exists before container startup.

Python static checks, shell syntax checks, CLI help, TOML parsing and whitespace
checks passed. The helper suite simulates Slurm and Apptainer execution. An additional
check used real Apptainer 1.4.5 against a loopback mock registry: it reproduced the
reported tag-plus-digest rejection, then verified that all three corrected
digest-only references requested their exact pinned manifests. No image was
downloaded or built. A minimal real writable build was also attempted to validate
the cache bind fix, but this host lacks usable user namespaces and Apptainer
stopped before entering the build. The setup hooks were tested directly instead.
No live UBELIX deployment was changed. Cluster resource policies
and scientific runtimes still require the
[deployment acceptance check](../../ops/ubelix/README.md#deployment-acceptance-check).

Evidence is retained on the execution host under
`/common/dev/cedric/setup-logs/ubelix-checkout-20261029/`.
The image-reference regression and parser check are recorded under
`/common/dev/cedric/setup-logs/ubelix-image-refs-20261029/`.
Cache-bind checks are recorded under
`/common/dev/cedric/setup-logs/ubelix-cache-bind-20261029/`.
