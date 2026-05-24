# TODO: Hyperparameter Tuning

Goal: add first-class hyperparameter tuning by reusing controller tasks and
grouped child runs. Trials remain ordinary GammaBoard runs with normal tasks,
snapshots, logs, measurements, and panels.

## Current State

Implemented:

- TOML templating for run/task/node-launch configs via top-level
  `replacements` and `$(name:default)` placeholders.
- Task-level evaluator selection.
- Optional accumulator moments through `moments = { max_order = 2|4 }`.
- Generic accumulator metric extraction and metric-targeted sample stop
  conditions.
- Task-local sample measurements persisted as `run_tasks.measurement_output`.
- Runtime-composed `time_normalized_variance`.
- Control-plane controller tasks that do not consume sampler/evaluator
  assignments.
- Grouped child runs via `parent_run_id`, `parent_task_id`, `spawn_kind`, and
  `spawn_label`.
- Recursive child-run deletion when a parent run is deleted.
- `parameter_scan` as the first child-run controller task.
- `hyperparameter_tuning` task spec with shared parameter domains:
  `float { min, max }`, `integer { min, max, step? }`, and
  `categorical { values }`.
- `random_search` optimizer with deterministic per-trial parameter generation.
- Basic tuning panels: progress, objective by trial, and trial table.
- E2E coverage for random-search child run creation and objective collection.

## First Version Shape

```toml
[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 4
max_failed_trials = 2
trial_run_toml = """
name = "trial-mu-$(mu_scale:0.5)-bins-$(bins:64)-$(subtraction_mode:\"auto\")"

[[task_queue]]
name = "sample"
kind = "sample"
measurement = { quantity = { metric = "time_normalized_variance" }, mode = "minimize" }
stop_condition = { max_samples = 1_000_000, relative_error = 0.01, metric = { name = "variance" } }
accumulator = { config = { kind = "scalar", moments = { max_order = 4 } } }
sampler_aggregator = { config = { kind = "havana_training", bins = "$(bins:64)" } }
"""

[task_queue.optimizer]
algorithm = "random_search"
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
```

Semantics:

- The objective reads the child task measurement selected by `source_task`.
- Trial precision and sample budget stay inside the child sample task
  `stop_condition`.
- `max_concurrent_trials` controls how many child runs are kept active.
- Failed child measurements become failed trials.
- Tuning fails if `failed_trials > max_failed_trials`.
- Random-search trials are deterministic from `(seed, trial_index)`, so the
  controller can reconstruct planned parameters after restart without a
  separate optimizer-state blob.

## Remaining Work

1. Improve objective selection.
   - The first implementation requires exactly one measurement result.
   - Add explicit component/result selectors for vector-valued objectives.
   - Fail with a focused config error when a child measurement is multi-result
     and no selector is provided.

2. Decide whether to add trial persistence.
   - Current state is reconstructed from child runs plus deterministic
     generation and compact `controller_output`.
   - A `hyperparameter_trials` table would make historical optimizer state and
     partial failed measurements easier to query.
   - Do this only if the JSON controller output becomes too weak for real
     optimizers.

3. Add more optimizers.
   - Add `grid_search` for deterministic exhaustive scans over discrete domains.
   - Add Bayesian/global optimizers only after the trial lifecycle is stable.
   - Keep optimizer-specific hyper-hyper-parameters under
     `optimizer.params`.

4. Polish panels.
   - Add best-so-far plot.
   - Include objective sample count in the trial table.
   - Show parameter columns individually when the number of parameters is small.
   - Link table rows to child runs consistently with parameter scan.

5. Add examples.
   - Cheap Symbolica tuning config with an obvious optimum.
   - One `time_normalized_variance` example using fourth moments.

6. Add failure-path tests.
   - Failed child measurement increments failed trial count.
   - Exceeding `max_failed_trials` fails the tuning task.
   - Parent-held workers are redistributed to tuning child runs.

## Deferred

- Trial pruning or early stopping beyond child task stop conditions.
- Penalized failed-trial observations via `failure_value`.
- Replicate trials per parameter point.
- Parameter-importance plots.
- 2D response surfaces.
- Heatmaps over vector components or histogram bins.
- Worker capability routing per trial.
