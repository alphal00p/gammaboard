# Operations

Use the same runtime config and port offset as the deployment. Run IDs avoid
ambiguity when several runs share a name.

## Manage work

```bash
./gammaboard run list
./gammaboard run inspect RUN
./gammaboard run task list RUN
./gammaboard run pause RUN
./gammaboard run resume RUN --max-evaluators 16
./gammaboard node list
./gammaboard node start-local 2
./gammaboard node stop -a
```

Workers belong to a root run's pool; controllers place them among children.
Assigning or pausing a child resolves to its root family. Pause checkpoints work
and retains pool membership; unassign/stop removes membership. Expired workers
are not live capacity. Launch history records connection attempts, not current health.

Graceful deploy shutdown records recreatable workers and intended assignments.
Start with `deploy --resume-workers` to recreate them; plain `deploy` leaves them
stopped. Use `node start-local`, dashboard launch requests, or the cluster launcher
so launch settings are recorded. Manually started `node run` workers cannot be
automatically recreated. A forced kill cannot save a new shutdown roster.

## Edit and duplicate

The definition editor offers templates, copy/download TOML, and duplication.
Every action uses the current draft. Pending integration tasks can be edited or
removed in place if references remain valid; active/completed tasks can only be
exported or duplicated. Controller-owned queues require a standalone run to edit.

Duplicate creates a **fresh execution**, without results, checkpoints, or workers.
Controller duplicates create new children; a child can become a standalone run.
External paths are retained, so choose distinct writable artifact paths for
independent runs. Historical snapshot branching is not supported.

```bash
./gammaboard run export RUN > run.toml
./gammaboard run duplicate RUN NEW_NAME
./gammaboard run task append RUN tasks.toml
./gammaboard run task export RUN TASK_ID > task.toml
./gammaboard run task duplicate RUN TASK_ID
./gammaboard run task edit RUN TASK_ID task.toml
./gammaboard run task remove RUN TASK_ID
./gammaboard run remove RUN
```

Removal drains the whole run tree before deleting it. Browser deletion continues
in the background; the CLI waits. If removal fails, inspect the error and retry
after workers stop. After a server restart, refresh the run list before retrying.

## Read performance

Overview, Diagnostics, and Graphs share **30 s · 5 min · All** or a dragged custom
interval. Activity is first in Overview. Accepted samples and allocated core-hours
are run totals; worker status, memory, report age, and queues are current.

The four busy fractions are **sampler compute**, **sampler I/O**, **evaluator
compute**, and **evaluator I/O**. They measure elapsed active time, including I/O
waits; completed operations awaiting collection do not count. Compute and I/O can
overlap. They are not CPU/GPU utilization or database saturation measurements.
Check reporting-worker coverage before interpreting an average.

Rates count accepted progress. Missing/stale reports are unavailable, not zero;
restarts and task changes start new measurement intervals. Allocated core-hours
are assigned wall time × declared CPU allowance (one if omitted). Memory is
scoped to reporting processes, not whole-host use. Queue/batch/I/O detail belongs
in Diagnostics; operation latencies overlap and cannot be summed into utilization.

Reporting defaults to two seconds; graph refresh is five seconds. Zoom shows
available detail but cannot recover sub-report spikes. Higher reporting frequency
increases database history: two-second cadence is 43,200 rows per worker/day.

RSD is standard deviation / mean absolute contribution; normalized ESS is
`(Σ|w|)² / (N Σw²)`, between zero and one. Both use the training projection.
GammaLoop Norm Mean is mean per-sample complex magnitude, not magnitude of the
final mean. Unsupported statistics display unavailable.

## Backup and recovery

```bash
./gammaboard db backup --output /safe/path/gammaboard.dump
# Stop servers and workers before replacing the database:
./gammaboard db restore /safe/path/gammaboard.dump
```

Restore replaces database contents; `--yes` acknowledges replacement in scripts.
Backups must match the current schema/model format. A database backup does not
include external model files or generated physics states.

Check **Checkpoint recovery** or `run inspect` for the last committed checkpoint.
Graceful pause/stop saves sampler state, accumulator, pending feedback, and progress
together. A crash restores that boundary and replays later work: live progress may
roll back, and evaluation/callbacks may run again. Invalid checkpoints fail visibly.

Accepted samples and ordered training feedback remain consistent with the restored
history. Claim tokens reject superseded results; an abandoned submission transaction
releases its locks after a fixed 30-second idle timeout. These guarantees assume a
healthy durable database and restorable sampler state, not arbitrary storage
corruption or irreversible external side effects.

External sampler artifacts must remain immutable and accessible while referenced
by any checkpoint/stage. GammaBoard stores references, not file contents or versions;
an overwritten MadNIS model cannot be repaired by restoring database JSON.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Port in use | Stop the conflicting service or select a port offset |
| Browser access rejected | Allow its exact origin in server config |
| No workers start | Launch-request errors, capacity, and external launcher availability |
| Run stops progressing | Task errors, worker leases, oldest unfinished batch, and database latency |
| Worker/task failure | Dashboard Logs, then local worker or Slurm stderr |
| Growing database | Performance-history volume, payload size, and retention needs |

Default logs: `resources/logs/nodes/`, `resources/db/logfile` (offset suffix when
used), and `logs/slurm/` on UBELIX. For live measurements:
`gammaboard --json run performance RUN --duration 60s --interval 1s`.
