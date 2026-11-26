# Operations

Use the deployment endpoint or its explicit instance/connection file. Run IDs avoid
ambiguity when several runs share a name. Commands and file ownership are in the
[CLI reference](local-cli.md). For run-card fields see
[configuration](config.md); for statistical definitions see [stratification](stratification.md).

## Dashboard navigation

- **Runs** shares a run selector across **Tasks**, **Summary**, **Performance**,
  and **Settings**. Search by name, ID, status or parent; **Load more runs** extends
  the catalog, and **Show child runs** includes controller children.
- **Tasks** contains the queue and the selected task's **Result**, **Observables**,
  **Stratum output**, and **Diagnostics**. Collapse the queue to give results more
  space. Unsupported tabs and actions stay visible but disabled, with explanations.
- **Summary** shows run identity, totals, targets and checkpoint recovery;
  **Settings** shows the effective evaluator/sampler configuration and source stage.
- **Performance** contains **Overview**, **Diagnostics**, **Graphs**, and **Queue
  tuning**. Queue tuning follows the active sampling task, not the task being
  inspected. It is disabled without active sampling; changing tasks discards its
  unsaved edits. Controller-owned child tasks are read-only here.
- **System** manages workers and displays database/server settings. **Logs** retains
  its filters and loaded older pages; use **Refresh** to request the latest page.

The URL records the selected run, task and tabs for bookmarks and Back/Forward.
Switching tabs retains loaded plots and view preferences while pausing hidden
polling. Returning shows cached data with a refresh indicator; failed refreshes
retain the last data with a warning. Worker lists and actions always recheck live
state. Plot preferences persist locally; result data and imported reference bundles
are page-session state. Expand result fields or table cells for full precision,
help and configuration details.

## Manage work

Workers belong to a root run's pool; controllers place them among children.
Assigning or pausing a child resolves to its root family. `gb run pause RUN`
checkpoints work and retains pool membership; `gb run start RUN` re-enables live
members. Unassigning or stopping a worker releases its membership. Expired workers
are not live capacity; launch history records attempts, not current health.

Graceful instance shutdown marks live workers **Saved for restart**, preserving
launch recipes and intended assignments. Workers stopped separately are not marked.
Use `gb worker restore` or `gb start --restore-workers` to request the saved fleet.
A launch through the CLI, dashboard or helper records a recipe; directly executed
workers without a recipe must be started by their owner.

**System → Workers → Offline** shows offline and starting workers:

| Action | Effect |
| --- | --- |
| **Start** | Request one worker using its saved recipe |
| **Restore saved workers** | Request only workers marked for restoration |
| **Cancel restoration** | Clear restoration intent while retaining the assignment |
| **Unassign** | Release the run reservation while retaining restoration intent |
| **Stop** (live worker) | Stop the worker and clear assignment/restoration intent |

These actions retain launch recipes and history. Bulk actions validate each captured
worker identity, skip ineligible rows and report individual failures. Stale rows
cannot modify replacement processes or pending launches. Registration consumes the
restoration marker. Launch reservations are tracked per worker.

Auto-assignment accounts for pending launches. Assigning a fresh sampler can release
an expired sampler reservation with no pending launch, including its restoration
intent. Live workers and queued launches retain their slots.

## Edit and duplicate

The definition editor offers templates, copy/download TOML, and duplication.
Every action uses the current draft. Pending integration tasks can be edited or
removed in place if references remain valid; active/completed tasks can only be
exported or duplicated. Controller-owned queues require a standalone run to edit.

Duplicate creates a **fresh execution**, without results, checkpoints, or workers.
Controller duplicates create new children; a child can become a standalone run.
External paths are retained, so choose distinct writable artifact paths for
independent runs. Historical snapshot branching is not supported.

Use the [run/task commands](local-cli.md#runs-and-tasks) to export, edit, duplicate
or remove definitions from the CLI.

Removal drains the whole run tree before deleting it. Browser deletion continues
in the background; the CLI waits. If removal fails, inspect the error and retry
after workers stop. After a server restart, refresh the run list before retrying.

## Read results and plots

Each component's result keeps its primary mean/error, target comparison, RSD and
absolute mean together. **Pooled** and **Stratified** identify the estimator;
**Independent sum** identifies a campaign result. Each pooled comparison has its
own mean and error. See [estimator terminology](stratification.md#estimator-terminology).

Parenthesized errors retain two uncertainty digits: `5.4123(25)` means
`5.4123 ± 0.0025`, while `1.2(3.5)` means `1.2 ± 3.5`. Any exponent applies to
both numbers. Expand a field for full precision. Unavailable errors remain
unavailable, never zero. The absolute mean is the mean of absolute component
values, not the absolute value of the signed mean.

A configured `target_error` appears as an orange **Target ±1σ** band around the
dashed target line. Its legend entry toggles both; the tooltip gives its error and
limits. Zero target error gives a line only. Target comparisons use both independent
uncertainties in quadrature; see [target configuration](config.md#integration-runs).

RSD measures relative sampling noise using the primary estimator. Normalized
training ESS describes concentration of the training weights, not confidence in
the integral. Their [definitions and limits](stratification.md#rsd-and-training-ess)
apply to both result summaries and history plots. The recorded training projection
appears beside ESS; missing historical information remains labelled as such.

Histories contain cumulative accumulator snapshots, not independent batch
estimates. Long histories are downsampled while retaining their endpoints; sample
count remains the canonical ordering. Zoom and legend choices change display only,
and the vertical range fits the visible interval and enabled curves. Recovery may
roll progress back and discard newer snapshots; the dashboard replaces stale
history. UI history is not a durable sampler checkpoint.

## Observable comparisons

Use the searchable **Observable** picker, previous/next controls or **All
observables** to choose a histogram. Its information button explains the estimator
and any omitted campaign observables. **Comparisons** holds imported references
and matching options; **Axes and ordering** holds scales and discrete sorting.

Observable histograms with frozen stratum statistics expose two compact selectors.
**View** defaults to **Combined · stratified**, offers **Combined · pooled**, and
can search any **Stratum (prefix)**, including ragged prefixes. Individual strata
show their conditional contribution. **Display** selects Estimate, Compare
estimates, Compare errors, or Error reduction. Comparisons add at most
one comparison curve for the same population. Switching the combined estimator
selects its own mean and error, including in exports; it never borrows the other
estimator's error. Error views use absolute standard errors, which remain useful
for integrals near zero, and hide value-only controls and uploaded value references.
Error reduction above one indicates improvement; zero or unavailable denominators
leave gaps. See the [statistical definitions](stratification.md#observable-statistics).
Selections persist with the histogram's view preferences and are included in
figure JSON/CSV exports.

**Upload Bundle** in both **Observables** and **Combined Observables** accepts
JSON and HwU files up to 16 MiB. JSON imports stay in the browser; HwU files are
converted by the authenticated server to the same comparison format. Imports
remain local to the current panel session and never change a run's accumulator.
HwU imports preserve bin edges, central values, errors, titles, axis flags and
TYPE descriptions. Extra named weight columns are omitted with a notice. Duplicate
titles receive numeric suffixes. **Add Histogram** selects the dropdown entry whose
displayed name exactly matches the selected observable's title in the table. If
there is no available match, it selects the first compatible histogram in the
bundle that has not already been added. HwU cannot recover original
sample counts, raw moments or category labels. Integer unit-width bins can be
compared to discrete observables by index; review the selected matching mode.

**Merge adjacent bins** changes only the display of continuous histograms. One
integer factor applies to the result and all selected comparisons, including the
comparison estimator in Compare estimates. The maximum factor is limited by the
shortest selected histogram. Compare errors and Error reduction keep their original bins.
Bins are merged in coordinate order, preserving a smaller final group. Overlaps
and merges across gaps are rejected. Underflow and overflow remain separate.
Values are bin integrals and are summed; for an imported differential density,
select **Density per unit x** to multiply its values and errors by bin width
before merging.

Merged-bin uncertainties use quadrature, an **uncorrelated-bin approximation**.
Cumulative uncertainties remain unavailable: GammaLoop histogram snapshots do
not retain cross-bin covariance.
Missing errors stay unavailable. Comparisons draw their own edges; ratios use only
matching intervals or exact sums of finer reference bins, never midpoint sampling
of a wider bin. Shared grouping does not guarantee aligned edges when the
original binnings differ.

Factors persist as panel view preferences across refreshes. **Reset** restores the
original binning and zoom; the input value convention stays selected. Bundle
exports retain original data, while figure exports use the displayed bins.

Both **Observables** and **Combined Observables** offer **JSON** and **HwU**
downloads of the entire observable bundle. JSON retains metadata and underflow/
overflow results; HwU contains the central value and uncertainty of each regular
bin. Campaign JSON uses GammaLoop-compatible histogram fields, with
`statistics_kind = "derived_moments"` and `estimator = "independent_sum"`: its moments
encode the sum of independent child estimates and their quadrature uncertainty.
Entry and diagnostic counters are actual sums. These derived moments are for
reading/plotting and are not pooled samples suitable for resuming or ordinary
accumulator merging. JSON export checks that the encoded moments reproduce values
and errors (relative tolerances 1e-12 and 1e-6 respectively); if float64 precision
cannot preserve an uncertainty, it reports an error and HwU remains available.
Campaign HwU writes values and errors directly. View settings and
uploaded comparisons do not change these data downloads.

Campaign downloads default to the aggregate only. Check **Include child observables**
to append the original unscaled child data: JSON adds a `children` array with
names, coefficients, source references, and bundles; HwU adds child-labelled
histogram blocks with coefficient comments. Children are fetched only when an
export is requested, and the aggregate is calculated from exactly those fetched
states. An active campaign's export may therefore be newer than the last dashboard
refresh. The same compatibility rule as the dashboard applies: only observables
present with matching layouts in every child enter the aggregate; omissions are
listed in JSON. Original child bundles can still contain those omitted observables.

**Download plots PDF** renders one observable per page, using the current scales,
bin merging, zoom, and enabled relative-error/ratio plots. When comparison bundles
are uploaded, check **Include comparisons in PDF** to include their overlays.
Configured selections are retained per observable; for observables not yet
configured, the export matches the table title to the comparison dropdown name,
falling back to the first compatible entry. Uncheck the option to export only the
run's observables. PDF generation shows progress and leaves the dashboard view
unchanged.

## Stratum plots and PDF inspection

**Stratum output** shows allocation and hit-rate histories during training. Frozen
inference adds component contributions, errors and variance shares. The component
selector applies to both convergence and variance plots. Legend selections and
**Show combined estimate** change visibility only. **Between snapshots** uses
count differences, so downsampling can widen its reporting intervals.

Create a [PDF diagnostic task](config.md#pdf-diagnostic-tasks) from a saved sampler
to inspect its proposal without changing its statistics. **Stratum** selects the
overall mixture or one searchable stratum. 2D slices show Target, Sampler and
Sampling mismatch together; 1D slices overlay Target/Sampler beside the auxiliary
curve. Both have fixed density-value and mismatch histograms below.

**Source integral reference** divides the absolute training projection by its
full-domain integral estimate and leaves the joint sampler PDF unchanged. The
reference disclosure shows the source, value, standard error and sample count.
Preference is a compatible configured target, the sampler source, then a compatible
accumulator from an earlier task. Later tasks cannot change this fallback. New
rasters retain source statistics in their checkpoints. Historical outputs remain
readable; missing projection/error metadata is not guessed.

For signed projections, a signed target is not an absolute-integral reference.
Use an explicit named target such as `target = { components = { abs_real = 1.2 } }`
and `target_error = { abs_real = 0.03 }`, or `abs_training_projection`. Nonnegative
`norm` and `norm_squared` projections also accept targets with those names.

**Slice** divides each field by its mean over the evaluated grid. Both relative
densities have mean one; this compares shape while removing allocation/overall
scale differences. Complete data and a positive mean are required. Zooming never
changes normalization. A selected stratum retains its joint PDF, including the
prefix probability and leakage, in Source mode. These are slices, not marginals.
Native Havana line PDFs retain their exact step edges.

Density plots share Viridis, Cividis or Inferno and linear/logarithmic scale.
Click a colorbar to set limits or return to Auto. Target/Sampler limits are linked
by default; logarithmic limits are entered in original density units. Spatial
zoom is shared and does not rescale colors. **Reset view** restores the extent.

Mismatch uses `log10(PDF / target)` by default, or `PDF / target - 1`. Red means
undersampling, pale green is zero mismatch, and blue means oversampling. Limits
may be asymmetric. Positive target/zero PDF and zero target/positive PDF retain
extreme colors; both-zero and missing values have separate counts. Zeros on a
logarithmic density plot are light grey. **Projection sign** is available for
signed projections; it cannot recover component signs from a norm or absolute
projection. Coverage and normalization disclosures contain additional details.

Histograms default to **Weight by sampler density**: each grid point contributes
its selected joint sampler density to both histograms. Turning this off gives
every grid point equal weight. Bin edges stay fixed when weighting is toggled,
including when extreme values have zero sampler density. Weighted histograms describe how values are
distributed under the proposal restricted to this slice, but they remain
deterministic diagnostics with no Monte Carlo errors. Zero-density regions have
zero weight; use unweighted histograms and Coverage to inspect them. Logarithmic value bins use their actual widths for density.
Nonfinite values (and zeros for logarithmic bins) are excluded and counted. Spatial
zoom does not change the histogram population.

**Export** selects plots, strata, format and current/full extent. PDF exports use
one plot per page; multiple SVG, PNG, JSON or CSV files are bundled in a ZIP.
Per-plot buttons use the same formats. JSON retains raw and displayed values,
reference metadata and exceptional mismatch states; CSV exports numeric tables.
Image exports retain plot scales and normalization labels. Large on-screen grids
use cached rasters; SVG/PDF exports render the underlying cells.

Stratum histories offer elapsed-time and reported sampler-time axes, follow-latest
and persistent zoom. Sampler time uses the latest preceding report for the same
task; missing timestamps stay absent. Classification always uses the complete
output vector, independently of the plotted training projection. Integrand
transforms contribute their Jacobians; custom process materializers are rejected
because these probes cannot reconstruct their coordinate mapping.

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

**GammaLoop time per sample** in task Diagnostics measures elapsed time inside
GammaLoop, averaged over the task's evaluated samples. It excludes GammaBoard's
queue/database work, sampler work and observable aggregation. Mapping is
GammaLoop's sampling map and channel partition, not the strata. Generated
evaluator calls and event processing are subsets of integrand time; Other integrand
work excludes both. Unclassified GammaLoop time is the residual after subtracting
mapping and inclusive integrand time from the total, not a separately timed operation.

## Campaign dashboard

Campaign Overview combines accepted progress and allocated core-hours across child
runs. Activity is weighted by reporting worker-time; throughput sums accepted
counter changes over one shared interval. Performance Diagnostics and Graphs use
a remembered child selection.

Task Diagnostics gathers the latest usable publishing-stage accumulator for each
child. Evaluation counters and times combine before fractions are calculated.
Retained extrema are ranked by their contribution to the combined estimate,
including child coefficients and stratum attempt counts. Compatibility groups and
missing-child coverage are explicit; absent diagnostics are not treated as zero.

Campaign views refresh while visible and retain the previous result during refresh.
Controller scheduling and aggregation continue independently of dashboard viewers.

Queue tuning defaults to all children and unfinished sample tasks. The selector
can restrict child names and publishing/non-publishing stages or a named task.
Only edited fields are applied. Durable rules also apply to children created later;
completed and failed tasks are skipped. A later edit takes precedence per field.
Clearing the fixed-size override restores that child's run default.

**Tasks → Scheduling** edits concurrency, evaluator requirements, allocation
windows, pilot minima, and stopping conditions. Changes serialize with the
controller and reject stale forms. Children finish already-generated samples and
queued evaluations, deliver feedback, and checkpoint. Unused reservations are then
released and assignments reconsidered. Draining can exceed a revised allowance or
cap; this work remains counted and no additional credit is issued above the cap.
A sampler call already in progress must finish before the drain can take effect.

Campaigns retain valid published data from failed children and continue scheduling
healthy children. Failed rows remain red. As soon as a child publishes a result,
the campaign displays a weighted subtotal labelled **Partial result — n/N children**.
Its error covers included contributions only; missing results are not measured
zeros. Partial results and results with unavailable uncertainties cannot satisfy
an error target. Histogram coverage is recorded individually and in exports.
Observables publish independently of the selected scalar measurement. Missing,
empty, or not-yet-estimable child histograms do not suppress other contributions;
incompatible populated bin layouts are still omitted. JSON retains normalized
values and null errors when uncertainties are not yet available; HwU requires
available uncertainties. Observable identity is its name, not its display title.

**Tasks → Edit children** provides two panels:

- **Tasks across children:** add one task from TOML or remove a pending task by
  exact name. The target dropdown defaults to all applicable children. Additions
  exclude failed children; removals preserve downstream references and at least one
  publishing task. Compatibility is previewed per child before confirmation.
  Definitions and eligibility are checked again transactionally; stale previews
  must be reviewed again. Confirmed requests are safe to retry.
- **Manage children:** create a fresh child from TOML, optionally starting from
  another child's current definition; delete a child after confirmation; or adopt
  a compatible standalone integration run from this database. Adoption retains its
  ID, tasks, results and checkpoints, counts its existing work, and transfers its
  worker pool. An active run drains and releases its old runtime first. A paused
  run with outstanding samples must be resumed before adoption. Pending worker
  launches and known shared accumulator ancestry prevent adoption. Deletion removes
  the contribution from future totals but retains spent samples and CPU time.
  The last remaining child cannot be deleted.

Integration-campaign children also expose the ordinary **Add tasks** button and
pending-task trashcan. Other controller-managed queues remain read-only. Appending
a task does not retry a failed task. Completed/failed campaigns remain read-only.
Copied definitions retain external sampler paths: choose distinct writable output
paths before creating an independent run. Configuration compatibility cannot prove
that two independently supplied samplers generated independent observations.

Current child queues and scheduling settings are included when exporting the
campaign definition. Queue-tuning rules remain runtime database settings.

## Backup and recovery

```bash
./gb db backup --output /safe/path/gammaboard.dump
./gb stop
# After the deployment owner confirms service termination:
./gb local db restore /safe/path/gammaboard.dump --yes
```

Restore replaces database contents; `--yes` acknowledges replacement in scripts.
Backups must match the supported [schema/model format](compatibility.md). A database
backup does not include external sampler files or generated physics states; retain
those immutable artifacts with the software/environment needed to read them.

Sample tasks checkpoint hourly by default and also at activation, graceful pause
and completion. Configure time/sample triggers with the [checkpoint policy](config.md#checkpoint-policy).
Check **Checkpoint recovery** or `run inspect` for the last committed boundary.
Long sampler callbacks can delay a save. A failed database save retains the previous
checkpoint and retries; a failed sampler commit notification does not undo a save.

Recovery restores the sampler, accumulator, pending feedback and queue boundary
together. Live progress can roll back and later work can replay; invalid checkpoints
fail visibly. Allocated core-hour accounting remains. Custom samplers must preserve
external artifacts required by saved stages and backups, even after receiving a
commit notification. The [recovery contract](execution-model.md#tasks-checkpoints-and-failures)
defines ordering, replay, notification retries and artifact responsibilities.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Port in use | Stop the conflicting service or choose distinct ports in the instance file |
| Browser access rejected | Update and reload the [browser-origin policy](deployment.md#authentication-and-browser-access) |
| No workers start | Launch-request errors, capacity, and external launcher availability |
| Run stops progressing | Task errors, worker leases, oldest unfinished batch, and database latency |
| Worker/task failure | Dashboard Logs, then local worker or Slurm stderr |
| Growing database | Performance-history volume, payload size, and retention needs |

Use `gb logs --service server`, `gb logs --service db`, or `gb logs --worker NAME`.
Local service/database files are under the configured `paths.log_dir`; Slurm job
logs are under the helper workspace's `logs/slurm/`. For live measurements:

```bash
gb --json run performance RUN --duration 60s --interval 1s
```
