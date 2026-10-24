# Run and task configuration

Run cards use TOML 1.1. Start from [run/task templates](../resources/templates),
then validate with `gammaboard run validate CARD`. Runner defaults are listed in
[run.toml](../src/config_defaults/run.toml); hosting settings are in
[deployment](deployment.md).

## Integration runs

Omitted `kind` means `integration`. Only integrations have a task queue.

```toml
name = "example"
target = { kind = "scalar", value = 1.0 }

[evaluator]
kind = "unit"
continuous_dims = 1

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 10000 }
accumulator = { config = "scalar" }
sampler = { config = { kind = "naive_monte_carlo", seed = 0 } }
```

The first evaluator establishes the run's domain; later evaluators must preserve
it. `evaluator_requirements` and `sampler_requirements` match worker capabilities
with `>=`, for example `sampler_requirements = { gpu = 1 }` at the run root.

### Evaluators and samplers

| Kind | Main settings |
| --- | --- |
| `unit` evaluator | `continuous_dims`, `discrete_dims`; installation/benchmark fixture |
| `symbolica` evaluator | `args`, tagged `source` with expression/constants or compiled-library settings |
| `gammaloop` evaluator | `state_folder`, integrand selection; dimensions inferred from state |
| `process_evaluator` | `command`, `domain`, `args`, `output`; [process guide](../process_api/README.md) |
| `naive_monte_carlo` sampler | Optional `seed` |
| `havana_training` sampler | `seed`, `bins`, `samples_for_update`, `initial_training_rate`, `final_training_rate` |
| `havana_inference` sampler | Trained source and optional `seed` |
| `process_sampler` | `command`, `args`; sampler defines its training windows |

For GammaLoop, regenerate states with the revision pinned by GammaBoard. Changing
channel definitions/order requires a new run and trained grid. `graph_groups`
selects graph groups from the loaded integration view. Optional `preprocessing`
runs ordered commands; `read_only = true` protects the loaded state by default.
`use_f128 = true` skips double-precision stability levels. See the
[GammaLoop card](../resources/templates/runs/gammaloop.toml) for field names.

## Task sources and training

Tasks have unique names within a run. For sampling, `evaluator`, `accumulator`,
and `sampler` accept:

| Source | Meaning |
| --- | --- |
| Omitted or `"latest"` | Continue from the preceding effective stage |
| `{ from_name = "earlier-task" }` | Restore that task's saved state/configuration |
| `{ config = ... }` | Start with explicit configuration; accumulator state is fresh |

Havana inference can import a selected trained grid:

```toml
[[task_queue]]
name = "inference"
kind = "sample"
stop_condition = { max_samples = 1000000 }
accumulator = { config = "scalar" }
sampler = { config = { kind = "havana_inference" }, from_stage = "training" }
```

Without `from_stage`, Havana inference uses the latest compatible prior stage.
A new seed starts a new random stream; omitting it continues the source stream.
Use `publish_result = false` on training and a fresh inference accumulator to
exclude training samples from published estimates. Unsupported handoffs fail.

| Other tasks | Purpose |
| --- | --- |
| `set_accumulator` | Establish/reset accumulator state without generating work |
| `pause` | Stop at a deliberate queue boundary |
| `image`, `plot_line` | Evaluate plane/line raster geometry |
| `pdf_adaptation_image`, `pdf_adaptation_plot_line` | Inspect a saved sampler's PDF |

`batch_transforms` inherits when omitted; `[]` clears it. Clear inherited transforms
on raster tasks that use direct geometry coordinates. Raster `discrete` selects
the branch; its dimensionality must match the geometry. Task append files accept
`[task]`, `[[task_queue]]`, or both (single task first).

## Accumulators and measurements

- `"scalar"` is shorthand for statistical vector component `value`.
- `vector` names components and a `training_projection`: `{ kind = "component",
  name = "real" }` or `{ kind = "norm" }`. Components must be nonempty, unique,
  and different from reserved `training_projection`.
- `moments = 2` is the default; `4` enables uncertainty estimates requiring fourth
  moments. Component projections reuse component statistics.
- `full_vector` retains every weighted value; size grows with sample count.
  Scalar collections can also return training feedback. Multi-component
  collections have no training projection and require feedback-free sampling.
- `gammaloop` retains native observable/histogram state and complex estimates.

Optional `discrete_projections` specifies `items` with `name`, `dims`, and
`fixed_dims`. `normalization` is `contribution` (default) or `conditional_mean`;
optional `streams` selects components. See the [projection example](../process_api/examples/python_scalar_sin/run.toml).

Sample stopping supports `max_samples`, `min_samples`, `absolute_error`, and
`relative_error`, with an optional metric selector.
Invalid values/overflow fail the entire batch and follow the retry policy;
they are never silently dropped or replaced with zeros.

Measurements default to every component's mean. To select one metric:

```toml
measurement = { selection = { kind = "metric", name = "variance", component = "real" } }
```

An omitted component selects the training projection. Scans/tuning also choose
`source_task`; tuning objectives use `mode = "minimize"` or `"maximize"`.

## Controllers

Controllers are root run kinds, each creating ordinary inspectable child runs.
They do not accept an integration task queue or a parent evaluator.

| Kind | Settings and example |
| --- | --- |
| `integration_campaign` | `children`, `allocation`, `stop_condition`, optional `target`; [card](../resources/templates/runs/integration-campaign-qft-like.toml) |
| `parameter_scan` | `parameters`, `child`, `measurement`, `max_concurrent_runs`; [card](../resources/templates/runs/parameter-scan-symbolica.toml) |
| `hyperparameter_tuning` | `parameters`, `child`, `objective`, `optimizer`, `max_concurrent_trials`; [card](../resources/templates/runs/hyperparameter-tuning-symbolica.toml) |

Child `run` is an inline TOML string or `{ file = "path.toml" }`. Relative references
are resolved and contents frozen at submission. Generated parameter values override
child `replacements`, which override the child card's defaults. A scan/categorical
source chooses exactly one of `values`, `linspace`, or `range`. Tuning supports
`grid_search` (no parameters), `random_search`, and `egobox` with typed `params`.

Campaigns sum independent child estimates with signed coefficients and combine
uncertainties in quadrature. Correlated estimates require covariance handling
outside this model. Campaigns follow each child's latest publishing sample stage;
error convergence waits for final publishing stages. Compatible histograms are
combined; incompatible layouts are reported as omitted.

`max_total_samples` is a hard cap on logical samples across every child stage,
including training and plotting. Error targets can finish earlier. A cap too small
for every final publishing stage fails explicitly. The [execution specification](execution-model.md#sample-budgets)
defines reservations, queued/buffered work, pilot grants, and retry accounting.

The default allocation favors expected variance reduction per second
(`weighted variance × throughput / samples`) after pilot coverage.
`largest_variance` ignores cost. Increase `min_samples_per_child` or
`allocation_window_samples` for steadier decisions; `max_active_runs` limits
concurrent children. Displayed variance contribution is not the allocation score.

## Batch and resource tuning

```toml
[sampler_aggregator_runner_params]
io_threads = 1

[sampler_aggregator_runner_params.queue]
target_batch_eval_ms = 2000
max_batch_size = 32768
max_generation_size = 262144
```

| Setting | Effect |
| --- | --- |
| `target_batch_eval_ms` | Adaptive evaluator batch duration; larger values amortize overhead |
| `max_batch_size` | Transport batch cap; larger batches can increase memory/database cost |
| `max_generation_size` | Sampler draw cap, independent of evaluator splitting; not a total memory cap |
| `fixed_batch_size` | Advanced override that bypasses timing adaptation |
| `io_threads` | Sampler I/O threads per runner; requires allocated cores and a runner restart |

The four queue size/time settings can be changed live through task `queue_tuning`
or the dashboard. Generation changes apply to the next draw. Training windows
remain sampler-owned and can span multiple draws; generation is split across
evaluators automatically. Leave other queue settings at their defaults unless
[benchmarks](benchmarking.md) justify a change. Threads do not allocate CPU cores.

## Replacements

Run, task-append, and node-launch cards support `replacements`; server/runtime
configs do not. `$(name:default)` as an entire string becomes the selected TOML
value; inside a larger string it is interpolated as text.

```toml
replacements = { samples = 10000, center = 0.5 }
# Inside the relevant task/evaluator tables:
# stop_condition = { max_samples = "$(samples:1000)" }
# source = { kind = "expression", expr = "(x - $(center:0.0))^2" }
```

## Stratified inference

Use `estimator = "pooled"` (default) while training a pipeline with
`sampler.config.stratification.kind = "voronoi"`. Set its stratum-only `domain`, prefix-keyed output-space `centers`
and optional positive component `scales`. The inference task selects
`estimator = "stratified"`, `from_stage = "train"`, and an explicit fresh
`accumulator = { config = ... }`. No user probability table is accepted.
The frozen sampler supplies it through `discrete_mass`. See the
[execution contract](execution-model.md#scaled-voronoi-stratification) and the
[tetrahedron run](../resources/templates/runs/tetrahedron-stratified.toml).

Symbolica can evaluate named real vectors using
`source = { kind = "expressions", components = ["x", "y", "z"], exprs = ["t", "2*t", "-t"] }`.
The accumulator component order must agree. Existing `kind = "expression"`
retains scalar or complex real/imag output. Stratification requires a statistical
scalar/vector accumulator or a GammaLoop-compatible observable producer satisfying
the masking contract; `full_vector` is unsupported. Ordinary raster tasks do not
attach a sampling partition. PDF-inspection tasks inherit the source partition
and inspect all its complete prefixes automatically.

With `accumulator = { config = "gammaloop" }`, Voronoi centres are pairs of
`[real, imag]`. For example, `domain = { rectangular = { strata_dims = 1, discrete_cardinalities = [2], continuous_dims = 0 } },centers = { "0" = [-1, 0], "1" = [1, 0] }` splits by real-part sign.
The primary integral is stratified; native histograms remain masked, full-weight
pooled estimates. Histogram totals need not equal the stratified central value.
Process observable callbacks must implement the protocol-v9 masking contract.

### Stratum plots and PDF inspection

Sample tasks with a partition expose a **Strata** panel. Training shows allocation
and hit-rate histories; conditional inference adds component contributions with
reported errors and a variance breakdown. The component selector applies to the
convergence plot and variance shares together. Legend selections and **Show total**
change visibility only. **Between snapshots** uses differences of retained counts,
so its intervals can become wider when history is downsampled.

Append a diagnostic task to a trained or frozen sampler:

```toml
[[task_queue]]
name = "stratum-pdf"
kind = "pdf_adaptation_plot_line"
sampler = { from_name = "inference" }
geometry = { offset = [0.0], direction = [1.0], linspace = { start = 0.0, stop = 1.0, count = 4097 } }
```

`geometry.discrete` specifies **integrand** discrete coordinates, excluding the
stratum prefix. Every complete prefix is inspected, including ragged/sparse labels.
The saved source determines the partition and training projection. Diagnostic
statistics start fresh and do not modify the source or publish a new integral.

Ordinary and stratified sources share the same **PDF comparison** panel. Its
**Scope** selector offers an overview, one stratum or comparisons paged four at a
time. Large prefix lists are searchable. The **View** selector switches between
density comparison, sampling mismatch, distributions, coverage and (when a
partition exists) membership. This keeps expensive plots and controls off screen
until requested.

The four normalization modes are:

- **Shape on this slice:** normalize the target by trapezoidal integration over
  the parameter grid. Native Havana line PDFs use their exact bin integrals;
  other PDFs use the same grid quadrature. A zero or unavailable integral has no
  normalized curve.
- **Source integral reference:** compare joint PDFs with the target divided by
  the source estimate of its absolute-projection integral, when available. A
  signed integral's magnitude is never used as a substitute.
- **Conditional density:** divide the joint PDF by the exact complete prefix
  mass. Unknown or zero masses produce unavailable conditional curves.
- **Joint density:** retain the sampler's returned PDF, including its prefix mass.

Mismatch and distribution views use shape or source-reference normalization so
that ratios compare like quantities. Raw density modes give the importance
target its own scale. **Normalization and leakage details** reports raw slice
integrals, applied divisors, exact requested masses, target shares, estimated
actual-cell masses and proposal leakage outside each intended cell. Leakage and
cell masses are grid estimates on the displayed slice, not full-domain guarantees.

The mismatch metrics are `PDF / target - 1` and `log10(PDF / target)`. Log ratio
is the default because near-zero targets can dominate a linear mismatch axis. **Coverage**
distinguishes positive target with zero PDF, zero target with positive PDF, both
zero, and unavailable data. Infinite/undefined ratios are counted explicitly and
can be located in Coverage. Histograms describe deterministic grid-point values;
they have no Monte Carlo error bars, and retain the shared histogram CDF, scale,
ratio and export controls.

Zoom/pan is linked within the panel and persists across refreshes and view
changes. **Reset view** restores the full extent. The expandable colour controls
share both bounds across comparable heatmaps, including logarithmic scales;
raw target and PDF quantities have separately labelled bounds. Native Havana
line densities are drawn at exact bin edges. Other sampler curves interpolate
queried points. Stratum boundaries are inferred from the classification grid.
The Strata history panel also has paged/searchable scope selection, elapsed-time
and reported sampler-time axes, follow-latest behaviour, and persistent zoom.
Sampler time uses the last preceding performance report for the same task;
missing timestamps are omitted rather than inferred from sample counts.

PDF leakage outside the intended stratum remains visible; only the target is
masked. Classification uses the complete output vector, independently of the
training projection being plotted. These views are slices, not marginals over
omitted coordinates. Custom process materializers are rejected because their
coordinate mapping cannot be reconstructed from a PDF query alone. Batch
transforms still apply, with their Jacobians included in the importance target.

Use [stratum-pdf-line.toml](../resources/templates/tasks/stratum-pdf-line.toml) with
the sine/tetrahedron examples. [strata-pdf-showcase.toml](../resources/templates/runs/strata-pdf-showcase.toml)
provides a complete signed two-dimensional example.
