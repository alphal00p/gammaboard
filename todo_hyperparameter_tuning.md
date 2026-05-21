# TODO: Hyperparameter Tuning

Goal: add first-class hyperparameter tuning while keeping trials transparent.
The optimizer should orchestrate ordinary GammaBoard runs/tasks, not hide work
inside opaque runner state.

## Current Assumptions

- Config templating is available for run/task/node TOML.
- Task-level evaluator selection is available.
- Inner-loop parallelism should use the existing GammaBoard run/task/node system.
- The outer-loop optimizer should only decide parameter points and consume
  completed measurement results.
- Trial execution should be reproducible from persisted submitted TOML plus
  replacement values embedded in that TOML.

## Proposed Shape

### 1. Generic Metrics And Stop Conditions

Before implementing a tuning task, add a generic measurement layer that can be
used by normal tasks and later by hyperparameter trials.

First milestone: done.

- Extend existing accumulators with optional higher-moment storage.
- Keep defaults compatible with current behavior and storage cost.
- Add backend-owned metric extraction from completed/current task state.
- Return metric values in one uniform shape: value, optional uncertainty,
  sample count, component name if vector-valued, and metric name.
- Generalize stop-condition evaluation so it can stop on a metric precision,
  not only on the integral estimate.
- Reuse this generalized stop logic for future measurement tasks and tuning
  trials.

Initial accumulator extension:

- `moments = "default"` keeps current behavior.
- `moments = { max_order = 2 }` supports mean/variance extraction.
- `moments = { max_order = 4 }` supports uncertainty estimates for variance.
- Vector accumulators apply the same moment policy per component.

Initial generic metrics:

- `mean`
- `abs_mean`
- `error`
- `relative_error`
- `variance`
- `relative_variance_error`
- `relative_squared_dispersion`

The tuning task should be built on this layer. Otherwise the optimizer would
need ad-hoc logic for extracting losses and deciding whether a trial measurement
is precise enough.

### 2. Measurement Config

Add an explicit measurement spec that defines what a trial is trying to estimate
and when that estimate is good enough.

Example:

```toml
[measurement]
source_task = "sample"
metric = "variance"
mode = "minimize"

[measurement.stop_condition]
relative_error = 0.01
max_samples = 1_000_000
```

For vector accumulators, use component-qualified metrics:

```toml
metric = { component = "real", name = "variance" }
```

### 3. Measurement Runtime

Implement measurement as backend-owned extraction from completed task state.
Do not scrape frontend panels.

Required work:

- Extract scalar/vector accumulator metrics from task queue-empty snapshots.
- Define uncertainty for variance estimates before allowing
  `measurement.stop_condition.relative_error` on `variance`.
- Support fallback sample-budget stops for metrics whose own uncertainty is not
  available yet.
- Store measurement value, uncertainty, sample count, source task id, and status.

Important constraint:

- A measurement is a result of a trial run/task, not a new sampler/evaluator
  execution mechanism.

### 4. Trial Run Template

A hyperparameter tuning task contains a nested run TOML template. Each trial
materializes one normal child run by applying replacements.

Example:

```toml
[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"

[task_queue.optimizer]
kind = "random_search"
seed = 1
max_trials = 64

[task_queue.parameters.mu_scale]
kind = "float"
min = 0.0
max = 1.0

[task_queue.parameters.subtraction_mode]
kind = "categorical"
values = ["auto", "none"]

[task_queue.measurement]
source_task = "sample"
metric = "variance"
mode = "minimize"
stop_condition = { relative_error = 0.01, max_samples = 1_000_000 }

trial_run_toml = """
name = "trial-mu-$(mu_scale:0.5)-$(subtraction_mode:auto)"
replacements = { mu_scale = "$(mu_scale:0.5)", subtraction_mode = '$(subtraction_mode:"auto")' }

[evaluator]
kind = "symbolica"
expr = "..."
args = ["x"]

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"
accumulator = "scalar"

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = "$(inner_max_samples:1000000)" }
sampler_aggregator = { config = { kind = "havana_training", bins = "$(bins:64)" } }
accumulator = "latest"
"""
```

### 5. Trial Persistence

Add explicit trial persistence rather than encoding optimizer state only in task
logs.

Required data:

- tuning task id
- trial id / sequence number
- trial run id
- parameters as JSON
- submitted trial TOML
- expanded/canonical trial identity if useful for display
- measurement value and uncertainty
- status: `planned`, `running`, `completed`, `failed`, `canceled`
- failure reason

Trials should link to normal run/task records so existing panels, logs, and
snapshots remain usable.

### 6. Optimizer Loop

Start with simple optimizers:

- `grid_search`
- `random_search`

Do not add `egobox` until trial lifecycle, measurement extraction, and failure
semantics are stable.

Outer-loop behavior:

- The tuning task proposes one or more parameter points.
- It creates child runs for those points.
- Existing node/task execution evaluates the child runs.
- The tuning task consumes completed measurements.
- The optimizer proposes more points until its stop condition is reached.

Parallelism:

- Support `max_concurrent_trials`.
- Parallelism means multiple child runs active at once.
- Do not parallelize inside the optimizer crate directly.

### 7. Failure Semantics

Initial policy:

- Failed child run means failed trial.
- Failed trial is not an optimizer observation by default.
- Tuning task fails if failed trials exceed `max_failed_trials`.
- Add `failure_value` later if a specific optimizer needs penalized failures.

### 8. Frontend

Minimal first view:

- Trial table: status, parameters, objective, uncertainty, run link.
- Best-so-far scalar plot.
- Active/failed trial counts.

Defer:

- Parameter importance.
- Multi-dimensional response surfaces.
- Optimizer-specific diagnostics.

## Implementation Plan

1. Generalize metrics and stop conditions.
   - Add optional higher-moment storage to existing scalar/vector accumulators.
   - Add one backend metric extractor for accumulator state.
   - Add a JSON-safe metric result shape with value, optional uncertainty,
     sample count, component, and metric name.
   - Extend stop-condition evaluation so it can target any extracted metric
     with `absolute_error`, `relative_error`, `min_samples`, and `max_samples`.
   - Keep current run/task behavior as the default when no metric target is set.

2. Add measurement specs on top of the generic metric layer.
   - Define `measurement.metric`, `measurement.source_task`, and
     `measurement.mode`.
   - Make measurement stop conditions use the generalized stop evaluator.
   - Store measurement value, uncertainty, sample count, source task id, and
     status.

3. Add trial persistence tables.
   - Store tuning task id, trial run id, parameters, submitted TOML, status, and
     measurement result.

4. Add `hyperparameter_tuning` task spec.
   - Parameter domains: float, integer, categorical.
   - Optimizer config: grid/random, seed, max trials, max concurrent trials.
   - Measurement config.
   - Nested trial run TOML.

5. Implement random/grid trial orchestration.
   - Generate trial TOML by injecting replacements.
   - Create normal child runs.
   - Poll child run/task completion through persisted state.
   - Persist observations.

6. Add e2e tests.
   - Random/grid search over a cheap Symbolica/unit example.
   - Verify child runs are created and linked.
   - Verify objective values are extracted.
   - Verify best-so-far updates.
   - Verify failure threshold behavior.

## Open Questions

- Should child runs be visually grouped under the tuning task or remain normal
  top-level runs with naming conventions?
- Should a tuning task be allowed to create child runs that require different
  worker capabilities?
- Should optimizing central value be allowed by default, or require an explicit
  `allow_biased_objective = true` style flag?
- Do we need trial pruning/early stopping, or is measurement stop condition
  enough for the first version?
- Should variance-relative-error require fourth moments, replicate trials, or
  both?
