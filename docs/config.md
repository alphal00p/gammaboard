# Run and task configuration

Run cards use TOML 1.1. Start from [run/task templates](../resources/templates),
then validate with `gb run validate CARD`. Runner defaults are listed in
[run.toml](../src/config_defaults/run.toml); hosting settings are in
[deployment](deployment.md). Dashboard controls and imports/exports are in
[operations](operations.md#read-results-and-plots).

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

Optional `target_error` gives independent, one-standard-deviation uncertainties
for the reference target. Use a number for a scalar target, or a partial map:

```toml
target = { kind = "vector", components = { real = 1.0, imag = 0.0 } }
target_error = { real = 0.02 }
```

Omitted errors are zero; supplied errors must be finite, nonnegative and refer to
target components. This also works for integration campaign targets. The signed
comparison pull is `(estimate − target) / sqrt(sampling_error² + target_error²)`;
target errors do not change sampling errors or stopping criteria. See
[reading results](operations.md#read-results-and-plots) for comparison details
and target bands in the dashboard.

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
[GammaLoop card](../resources/templates/runs/gammaloop-four-phases.toml) for field names.

For a precompiled Symbolica evaluator, provide a shared library and its exported
function name. Fixed bindings are separate from sampled arguments:

```toml
[evaluator]
kind = "symbolica"
args = ["x"]
[evaluator.source]
kind = "compiled"
path = "artifacts/eval.so"
name = "eval"
compiled_args = ["scale", "x"]
bindings = { scale = 2.0 }
```

The library must use Symbolica's complex evaluator ABI; it is not an arbitrary C
function. Generate it with the compatible Symbolica version. Relative paths are
resolved through resource roots. The compiled evaluator and fixed bindings have
an automated regression test; no generated physics evaluator is bundled.

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

Raw `image` and `plot_line` tasks accept `accumulator = { components = ["x", "y", "z"] }`
for named vector outputs. The compact `"scalar"` form means `["value"]`; `"vector"`
means the complex pair `["real", "imag"]`. Images provide one selector for components
and the full vector magnitude; complex phase is offered only for real/imag output.
Use a task-local evaluator projection, as in the tetrahedron's `xy-phase` task,
when a complex view is wanted for part of a higher-dimensional output.

### Checkpoint policy

Set periodic recovery triggers inside a sample task:

```toml
checkpoint = { interval_seconds = 3600.0, interval_samples = 0 }
```

The default saves hourly. Zero disables that trigger; either positive threshold
can trigger a save. For example, `interval_seconds = 0` and
`interval_samples = 10000000` saves after ten million newly completed task samples,
including zero contributions. Setting both to zero disables periodic saves but
retains activation, pause and completion checkpoints. Inherited accumulator samples
do not count toward the new task's sample interval. See [recovery semantics](execution-model.md#tasks-checkpoints-and-failures)
and [backup/restore commands](operations.md#backup-and-recovery).

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
optional `streams` selects components. See the [projection example](../process_api/examples/python_evaluator/run.toml).

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
| `integration_campaign` | `children`, `allocation`, `stop_condition`, optional `target`; [card](../resources/templates/runs/tetrahedron-campaign.toml) |
| `parameter_scan` | `parameters`, `child`, `measurement`, `max_concurrent_runs`; [card](../resources/templates/runs/tetrahedron-scan.toml) |
| `hyperparameter_tuning` | `parameters`, `child`, `objective`, `optimizer`, `max_concurrent_trials`; [card](../resources/templates/runs/tetrahedron-tuning.toml) |

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
`allocation_window_samples` for steadier decisions. Displayed variance contribution
is not the allocation score.

```toml
[allocation]
max_active_runs = 4
min_evaluators_per_run = 2
allocation_window_samples = 100000
min_samples_per_child = 10000
algorithm = "variance_reduction_rate"
```

`max_active_runs` defaults to 1 and is a ceiling. `min_evaluators_per_run` defaults
to 2 and must be positive. An active child reserves one sampler and at least that
many evaluators. Automatic assignment chooses complete compatible worker bundles,
then distributes remaining evaluators. Six interchangeable free workers therefore
support two samplers and four evaluators, even with `max_active_runs = 4`. Existing
operator-assigned pool roles are preserved. Insufficient resources leave children
waiting; changing the ceiling does not launch workers or convert assigned roles.
The campaign summary shows the ceiling, assigned roles and available concurrency.

`allocation_window_samples` is a **per-child scheduling quantum**, counted across
all child stages, including training. Each child keeps its workers until its own
quantum drains. Only that slot is reconsidered; other children keep progressing.
Pilot coverage has priority, then the allocation score decides between the
incumbent and waiting children. Equal scores keep the incumbent. Continuing a
child renews its quantum without restarting its sampler. Switching checkpoints
its sampler/statistics and preserves the workers of other retained children.

Applying campaign scheduling settings immediately rechecks assignments using the
new concurrency, resource requirements and priorities, even during an unfinished
quantum. Workers checkpoint before switching. Existing grants remain reserved
and can resume later; changing the quantum size affects future grants. A changed
stopping condition is rechecked too, including whether draining is still needed.

Every campaign uses durable cumulative quantum grants, including campaigns with
only an error target. Grants also enforce `max_total_samples` when specified.
Error convergence stops granting new quanta, finishes outstanding grants, and
rechecks the target before publishing completion. Final publication waits for
worker checkpoints. Interrupted grants survive pause, worker loss and controller
recovery; they are not issued a second time.

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
[statistical specification](stratification.md) and the
[tetrahedron run](../resources/templates/runs/tetrahedron-native.toml).

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
The primary integral and every observable bin use conditional per-stratum statistics.
Process observable callbacks return unweighted per-sample contributions under protocol v10;
GammaBoard applies masking and weights centrally.

## PDF diagnostic tasks

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

For two-dimensional inputs, the [native tetrahedron card](../resources/templates/runs/tetrahedron-native.toml)
includes complete line and plane PDF tasks that can be copied into another run.
[PDF inspection](operations.md#stratum-plots-and-pdf-inspection) describes display
controls; the [probe contract](stratification.md#stratum-plot-data-and-pdf-probes)
defines densities, normalization and supported coordinate mappings.
