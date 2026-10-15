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
