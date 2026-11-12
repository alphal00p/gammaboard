# Operations

Use the same runtime config and port offset as the deployment. Run IDs avoid
ambiguity when several runs share a name.

## Dashboard navigation

The header shows the connection status and last successful server-status update
across all tabs. The persistent tabs are **Runs**, **System**, and **Logs**. Runs shares one run
selector across **Tasks**, **Summary**, **Performance**, and **Settings**.
Top-level navigation is centered on wider screens; section and view tabs align
with the left edge of their content. Nested tabs use progressively quieter styles.
Tasks separates the selected task's Result, Observables, Stratum output,
and Diagnostics. Unsupported output tabs remain visible but disabled; Stratum output
becomes available once the task provides stratum data. An unavailable retained
selection displays Result. Collapse the queue to give the output more space;
the selected task's name, state, and kind remain above its output.
Worker actions remain visible but disabled when inapplicable, with hover explanations.
Completed runs cannot receive workers, but remaining pool members can still be removed.
Sample progress is an unlabeled bar with ETA; sample counts remain in the task table.
Campaign progress retains a full-width bordered panel, with its label and sample
count on the same row.
Changing tasks reserves output height during loading to avoid scrolling the page.
Summary combines identity, lifecycle, totals, and targets in a run overview, followed
by the selected task's metadata and expandable checkpoint recovery. Recovery errors
remain visible when collapsed. Settings shows effective evaluator/sampler configuration,
its source stage, with evaluator/sampler summaries before their configuration trees.
Run definition and deletion actions sit at the right of the worker-assignment
row below the run selector; child-run worker information occupies the same row.
Child runs retain their dash and an extra indent
in run-selection menus. New run and Show child runs sit beside the selector on wider screens.

Performance provides Overview, Diagnostics, Graphs, and Queue tuning. Queue tuning
automatically follows the active sampling task, independently of the task selected
for viewing results. There is no task selector; when sampling is not active the tab
is disabled. Task changes reset the form and discard unsaved edits. Live Queue and
Batch panels appear only for the active sampling task. Managed child tasks remain
read-only. System contains Workers, Database (runtime/server details and
destructive controls), and Settings (configuration paths and documentation).
Logs retains its own run and worker filters.

**GammaLoop time per sample** in task Diagnostics measures elapsed time inside
GammaLoop, averaged over the task's evaluated samples. It excludes GammaBoard's
queue/database work, sampler work and observable aggregation. Mapping is
GammaLoop's sampling map and channel partition, not the strata. Generated
evaluator calls and event processing are subsets of integrand time; Other integrand
work excludes both. Unclassified GammaLoop time is the residual after subtracting
mapping and inclusive integrand time from the total, not a separately timed operation.

Node management's **Stop Idle nodes** requests shutdown only for live nodes with
no current role, pending assignment, or pool membership. Eligibility is checked
atomically by the server. **Unassign all** reports only live nodes whose stored
assignment changed; expired stored assignments are also cleared but are excluded
from the notification count.

The URL records the selected run, task, and tabs for refresh, bookmarks, and
browser Back/Forward. Switching tabs keeps drafts, imported comparisons, and view
preferences while pausing inactive polling and chart rendering. Imported
comparisons and unsaved drafts remain browser-session state, not saved run data.

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

**System → Workers → Offline / suspended** lists saved worker identities,
reservations, last-seen times, and launcher availability. **Resume** queues the
original launch recipe under the same name. **Release assignment** frees the run
reservation while keeping the launch recipe. **Retire** also cancels automatic
resumption. Live identities and pending launches cannot be changed through stale
offline rows. Local resume respects the server's local-launch setting.
The Offline tab's toolbar offers **Resume all**, **Release all**, and **Retire all**.
Bulk operations validate each captured worker identity, skip unavailable workers,
and report individual failures. Release and retirement are safe to repeat.
Launch reservations are per worker: an old group request still marked `starting`
does not block a worker that already connected and subsequently stopped. Workers
still awaiting their first registration for that launch remain protected.

Assigning a fresh sampler to a standalone run atomically releases expired sampler
reservations that have no pending launch, including their automatic-resume intent.
Live workers and queued/starting launches retain their slots; auto-assignment
counts pending launches when planning capacity. Ordinary pool resume re-enables
only live members; it does not recreate offline processes.

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

Each percentage is a **time-weighted average across reporting workers**:
`100 × sum(active seconds) / sum(observed worker seconds)` within the selected
interval, using valid report spans and proportional clipping at its edges.
Longer reporting spans carry more weight; allocated CPU counts are not part of
the denominator. Compute includes materialization,
evaluation and reduction on evaluators, and generation, feedback/training and
aggregation on samplers. These are elapsed operation times, including internal
waits. I/O includes active queue/database operations and their waits; waiting for
new work is idle. Overlapping operations count once within each category, but
compute and I/O overlap independently.

For expensive integrands, high evaluator compute with low exposed waits usually
indicates well-fed workers.
Low compute with high fetch wait suggests insufficient input supply; high submit
wait suggests result-publication delays or backpressure. High I/O alone can be
healthy overlap. Low compute and I/O while work remains suggests idle/coordination
time. Compare accepted samples/s and the fetch/submit waits in Diagnostics; inspect
queue depth while the run is live. Current queues cannot explain a past interval
by themselves.

Rates count accepted progress. Missing/stale reports are unavailable, not zero;
restarts and task changes start new measurement intervals. Allocated core-hours
are assigned wall time × declared CPU allowance (one if omitted). Memory is
scoped to reporting processes, not whole-host use. Queue/batch/I/O detail belongs
in Diagnostics; operation latencies overlap and cannot be summed into utilization.

Reporting defaults to two seconds; graph refresh is five seconds. Zoom shows
available detail but cannot recover sub-report spikes. Higher reporting frequency
increases database history: two-second cadence is 43,200 rows per worker/day.

RSD is `sqrt(N) × SE(estimate) / |estimate|`, using each signed component of the
primary integral. Stratified runs use their conditional-estimate error, not the
pooled reference error. RSD is dimensionless and stays roughly constant during
inference with a fixed proposal. Cancellation to a zero estimate produces an
infinite RSD when the error is nonzero; `0/0` is undefined. Insufficient coverage
leaves it unavailable. Only finite values enter the RSD history plot.

Normalized training ESS is `(Σ|w|)² / (N Σw²)`, between zero and one, where `w`
is the weighted **training projection**. For partitioned sampling, calculate it
within each requested stratum, counting mismatches as zero contributions. The
aggregate is `Σ(N_s × ESS_s) / ΣN_s`; stratum-specific values appear under
**Stratum output**. This measures training-weight concentration, not the variance
of the primary stratified integral, and is not inferred from RSD.

Result uses the same compact fields for pooled and stratified estimators. Mean
histories are followed by the RSD and ESS plots. Each component keeps its mean,
target comparison, and RSD together in one column. Parenthesized uncertainties
use last-digit notation with two uncertainty digits retained: `5.4123(25)` means
`5.4123 ± 0.0025`, and `1.23(20)` means `1.23 ± 0.20`. Same-order errors retain
their decimal point: `1.2(3.5)` means `1.2 ± 3.5`; any exponent applies to both.
Target deltas and pulls use at most three significant digits and three decimal
places. Each component's absolute mean and its own uncertainty appear beneath
its RSD. Absolute means use the primary estimator, including per-stratum
allocation; older stratified snapshots without absolute sums show unavailable.
There is no separate absolute-mean RSD or progression plot.
The training projection recorded with the samples appears beside normalized
training ESS. Historical GammaLoop snapshots that omit it are explicitly marked
as unrecorded, with the absolute-magnitude fallback used by their ESS calculation.
Per-stratum estimates, counts, coverage, probabilities, and ESS live under
Stratum output. Optional moment fields preserve compatibility with old snapshots;
ESS remains unavailable when the needed historical training moments are missing.

Expandable fields use a triangle between the title and value. Click the title or
value, or focus the field and press Enter/Space, to expand precision, definitions,
or configuration children. RSD and ESS each carry their own definitions. Table
cells put the same triangle directly before the value; sortable headers and
editable controls retain their usual interactions. Expanded configuration trees
preserve nested objects, arrays, indentation, and expansion state across refreshes.
Observables use a searchable picker with previous/next navigation; the full table
is available under **All observables**. Upload Bundle stays on the left and
JSON/HwU/PDF exports on the right of the same action row.
The picker remembers the typed query across refreshes and focus changes. While
unfocused it displays the selected observable; focusing restores the query.
Typing filters results without changing the selection. Choosing a result, a
table row, or a navigation arrow changes the selected observable.
Their top-right information icon opens the estimator and campaign-combination
notes; import/export errors and omitted-observable warnings stay visible.
Matching uploaded references and the **Comparison** plot appear by default.
Matches use a compatible histogram name or an unambiguous title, and explicit
empty selections or hidden comparison plots are retained. Reference selection,
density inputs, and discrete matching live under **Comparisons**; axis scales
and discrete ordering live under **Axes and ordering**.
One **Merge adjacent bins** control applies the same grouping to the result and
all selected references (and the basic estimator in Compare estimates). The
largest permitted factor is limited by the shortest selected histogram. This is
a display transformation of the original bins; merged errors retain the
uncorrelated-bin approximation. Error/error-ratio views retain their original
bins. PDF plots use the same selections and grouping as the dashboard.
The merge-factor up/down buttons each occupy half of the input's height;
keyboard arrow keys and direct entry remain available.

Result histories are cumulative accumulator snapshots, not independent batch
estimates. Current panels load before history, and Observables is fetched only
when selected. Active tasks poll every three seconds; hidden sections pause
polling. Completed tasks refresh once when revisited. The history query selects
up to 256 snapshot IDs across the entire retained history, keeping the first and
last, before loading compact moment data. New samples append by sample count;
large gaps and deleted cursors after recovery trigger a fresh overview. Browser
history is bounded and retains its endpoints. A rollback cannot recover snapshots
that were deleted by recovery, and cached pre-rollback history is replaced.

The additive plot-snapshot migration stores compact moments separately from full
observables, excluding histogram bundles, discrete bins, sample coordinates and
evaluation diagnostics. Existing rows are projected on demand; migration does
not rewrite historical payloads. Each selected snapshot is decoded once and only
history panels are projected. Full observables and recovery state are unchanged.

Result, Observables, and read-only run panels (summary, configurations and queue
telemetry) use a page-session LRU cache, bounded to 64 entries/64 MiB with a
30-minute expiry. Returning to a tab displays its cached data immediately with
a small refresh indicator; a failed refresh keeps the last data with a warning.
Authentication failures and missing resources invalidate cached data. Login,
logout, database reset and completed run/task deletions clear the cache. Zoom
preferences remain in localStorage; result payloads do not survive page reload.
Settings retain their loaded values and refresh on return. Logs and historical
performance graphs retain their existing incremental/window caches. Health,
worker lists, assignments, task queues, permissions and action results continue
to use fresh requests and server-side validation.
Loading indicators occupy a fixed slot to the left of the nearest tab row;
their descriptive labels are available on hover and to assistive technology.
Single- and multi-series progression plots share automatic vertical range fitting
to the visible horizontal interval and enabled legend entries. Explicit fixed
ranges, such as worker utilization percentages, remain fixed.
Mean uncertainty bands render in one pass with the mean line, avoiding the
incremental painting delay on long histories. UI snapshots are distinct from
durable sampler checkpoints: recovery rolls back progress and deletes snapshots
after the checkpoint boundary while retaining allocated core-hour accounting.

Sampler batch insertions preserve explicit PostgreSQL abort codes. Lock timeouts
(`55P03`), deadlocks (`40P01`), and serialization failures (`40001`) retry the whole
insertion transaction with the same batch IDs, payloads, and feedback flag.
Retries use exponential backoff with jitter, capped at five seconds. The failed
bundle remains an ordering barrier; worker lease/control checks run between
attempts. These errors do not restart the sampler or reset its progress. Other
insertion errors, including connection loss with an unknown commit outcome,
retain the conservative checkpoint-recovery behavior. These retries are separate
from evaluator `max_batch_retries` and do not re-evaluate completed samples.

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

The [execution specification](execution-model.md#tasks-checkpoints-and-failures)
defines claim fencing, ordered feedback, rollback, replay, and external artifact
requirements. It is the interface contract for custom workers. An abandoned
submission transaction releases its locks after a fixed 30-second idle timeout.

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
