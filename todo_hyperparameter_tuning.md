# TODO: Hyperparameter Tuning

Goal: add first-class hyperparameter tuning by reusing the now-existing
controller-task and child-run machinery. Trials should remain ordinary grouped
GammaBoard runs with normal tasks, snapshots, logs, measurements, and panels.

## Current State

Implemented building blocks:

- TOML templating for run/task/node-launch configs via top-level
  `replacements` and `$(name:default)` placeholders.
- Task-level evaluator selection.
- Optional accumulator moments through `moments = { max_order = 2|4 }`.
- Generic accumulator metric extraction.
- Metric-targeted sample stop conditions through `stop_condition.metric`.
- Task-local sample measurements through `measurement = { quantity = ..., mode = ... }`.
- Persisted `run_tasks.measurement_output` for completed sample tasks.
- Runtime-composed `time_normalized_variance`, using variance and sampler
  throughput.
- Controller tasks that run in the control plane and do not consume a worker
  assignment.
- Grouped child runs via `parent_run_id`, `parent_task_id`, `spawn_kind`, and
  `spawn_label`.
- `parameter_scan`, which spawns child runs from `trial_run_toml`, injects one
  1D parameter, reads child measurement output, and exposes progress/table/plot
  panels.
- Frontend grouping of child runs under parent runs, plus basic scan panels.
- Parent run deletion recursively removes grouped child runs and clears node
  assignments for the deleted run family.

This means hyperparameter tuning should be an incremental controller task, not a
new execution model.

## Ready For Tuning

The prerequisite cleanup is done:

- Selected pending task output keeps polling until terminal state.
- Progress panels update through normal panel replacement.
- Child runs are grouped under parent runs in the run selection UI.
- `parameter_scan` e2e covers worker redistribution and live progress.
- A focused e2e covers recursive child-run deletion with parent cleanup.
- `AGENTS.md` documents controller tasks as control-plane work.

Next step: implement the tuning task core types and trial persistence.

## Recommended First Version

Add a new controller task:

```toml
[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 4
max_failed_trials = 2

[task_queue.optimizer]
kind = "random_search"
seed = 1
max_trials = 64

[task_queue.objective]
source_task = "sample"
mode = "minimize"
quantity = { metric = "time_normalized_variance" }

[task_queue.parameters.mu_scale]
kind = "float"
min = 0.0
max = 1.0

[task_queue.parameters.bins]
kind = "integer"
min = 16
max = 128

[task_queue.parameters.subtraction_mode]
kind = "categorical"
values = ["auto", "none"]

trial_run_toml = """
name = "trial-mu-$(mu_scale:0.5)-bins-$(bins:64)-$(subtraction_mode:\"auto\")"

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"
accumulator = { kind = "scalar", moments = { max_order = 4 } }

[[task_queue]]
name = "sample"
kind = "sample"
measurement = { quantity = { metric = "time_normalized_variance" }, mode = "minimize" }
stop_condition = { max_samples = 1_000_000, relative_error = 0.01, metric = { name = "variance" } }
sampler_aggregator = { config = { kind = "havana_training", bins = "$(bins:64)" } }
"""
```

Shape decisions:

- The objective references the child task measurement via `source_task`.
- Trial precision/budget stays inside the child sample task `stop_condition`.
- `max_concurrent_trials` controls how many child runs the controller keeps
  active.
- The tuning controller consumes completed child measurements only.
- Failed child runs become failed trials and are not optimizer observations by
  default.
- Optimizing central value is allowed; users are responsible for choosing a
  meaningful objective.

## Implementation Steps

1. Factor shared controller utilities.
   - Extract reusable child-run listing, child-run creation, assignment
     redistribution, and measurement loading from `parameter_scan`.
   - Keep the abstraction small; avoid a generic framework until the tuning task
     needs it.

2. Add tuning core types.
   - `HyperparameterTuning` task spec.
   - Parameter domains: `float`, `integer`, `categorical`.
   - Optimizer configs: `grid_search`, `random_search`.
   - Objective spec: same source-task wrapper shape as parameter scan, but with
     explicit `quantity` and `mode`.
   - Validate finite bounds, non-empty categorical values, positive
     `max_trials`, positive `max_concurrent_trials`, and non-empty
     `trial_run_toml`.

3. Add trial state persistence.
   - Prefer a real `hyperparameter_trials` table over only storing JSON in
     `controller_output`.
   - Store: tuning task id, sequence number, child run id, parameters JSON,
     status, measurement output, objective value, uncertainty, failure reason,
     created/updated timestamps.
   - Keep `controller_output` as a compact panel/read-model summary.

4. Implement random/grid orchestration.
   - Generate parameter points deterministically from optimizer config.
   - Create child runs with replacement values.
   - Reassign parent-held workers to active child runs.
   - Read child `measurement_output`.
   - Update trial status and best-so-far.
   - Complete when optimizer budget is exhausted and all active trials are
     terminal.
   - Fail when failed trials exceed `max_failed_trials`.

5. Add backend panels.
   - Progress: completed/running/failed/total trials.
   - Trial table: trial id, status, parameters, child run link, objective,
     uncertainty, samples, failure reason.
   - Best-so-far plot: trial sequence on x-axis, best objective on y-axis.
   - Objective scatter/line plot for 1D numeric searches when applicable.

6. Add tests.
   - Unit tests for parameter domain parsing and deterministic point generation.
   - Store tests for trial persistence.
   - Controller tests for trial lifecycle and failure thresholds.
   - E2E random/grid tuning over a cheap Symbolica example.
   - E2E worker redistribution: assigning workers to the parent tuning task
     must move useful work to child runs and update progress live.

7. Add an example config.
   - Cheap Symbolica objective whose optimum is obvious.
   - Use `time_normalized_variance` in one example and central value in another
     only if it clarifies the API.

## Deferred

- Bayesian/global optimizers such as `egobox`.
- Trial pruning or early stopping beyond the child task stop condition.
- Penalized failed-trial observations via `failure_value`.
- Replicate trials per parameter point.
- Parameter-importance plots.
- 2D response surfaces.
- Heatmaps over vector components or histogram bins.
- Worker capability routing per trial.

## Open Checks

- Confirm whether `relative_error` on variance always requires
  `moments = { max_order = 4 }` and fails clearly otherwise.
- Confirm `time_normalized_variance` has acceptable semantics when the latest
  sampler performance snapshot is missing or stale.
- Decide whether trial state must survive task spec edits, or whether tuning
  tasks are immutable once active like current run tasks.
- Decide whether `parameter_scan` and `hyperparameter_tuning` should eventually
  share a public `trial_run_toml` vocabulary, or stay separate but similar.
