# TODO: Hyperparameter Tuning

Goal: keep hyperparameter tuning as a transparent controller-task workflow.
Trials are grouped child runs with normal tasks, snapshots, logs,
measurements, and panels.

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
- Objective selection by measurement metric and optional component selector.
- `random_search` optimizer with deterministic per-trial parameter generation.
- `grid_search` optimizer over finite integer/categorical domains.
- Basic tuning panels: progress, best trial, objective/best-so-far plot, and
  trial table with run links, samples, objective uncertainty, and parameter
  columns.
- Example templates for error, grid-search, and `time_normalized_variance`
  Symbolica tuning.
- E2E coverage for random-search and grid-search child run creation and
  objective collection.
- E2E coverage for failed child measurements and parent-held worker
  redistribution.

## Current Shape

```toml
[[task_queue]]
name = "tune"
kind = "hyperparameter_tuning"
max_concurrent_trials = 4
trial_run_toml = """
name = "trial-bins-$(bins:64)-$(subtraction_mode:auto)"

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

[task_queue.optimizer.params]
max_trials = 64

[task_queue.objective]
source_task = "sample"
mode = "minimize"
quantity = { metric = "time_normalized_variance" }

[task_queue.parameters.bins]
kind = "integer"
min = 16
max = 128
step = 16

[task_queue.parameters.subtraction_mode]
kind = "categorical"
values = ["auto", "none"]
```

Semantics:

- The objective reads the child task measurement selected by `source_task`.
- Trial precision and sample budget stay inside the child sample task
  `stop_condition`.
- `max_concurrent_trials` controls how many child runs are kept active.
- Failed child measurements fail the tuning task in the first version.
- Random-search trials are deterministic from `(seed, trial_index)`.
- Grid-search trials deterministically enumerate finite integer/categorical
  domains. Float grids are deferred until there is an explicit discretization
  syntax.
- Trial persistence remains deferred; current state is reconstructed from child
  runs plus deterministic generation and compact `controller_output`.

## Immediate Work

1. Run the ignored full-stack tuning e2e tests once the local PostgreSQL test
   service is available.

2. Decide the next optimizer.
   - `egobox` is the likely next candidate for mixed continuous/discrete
     Bayesian/global optimization.
   - Keep the public config shape algorithm-neutral:
     `optimizer.algorithm` plus `optimizer.params`.

## Deferred

- Trial persistence tables.
- Float grid discretization syntax.
- Trial pruning or early stopping beyond child task stop conditions.
- Penalized failed-trial observations via `failure_value`.
- Replicate trials per parameter point.
- Bayesian/global optimizers.
- Parameter-importance plots.
- 2D response surfaces.
- Heatmaps over vector components or histogram bins.
- Worker capability routing per trial.
