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
name = "trial-mu-$(mu_scale:0.5)-bins-$(bins:64)-$(subtraction_mode:auto)"

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



Findings**
- `PanelCollection.jsx` is the biggest complexity hotspot at ~1900 LOC. It owns panel composition, PDF special cases, ECharts charts, canvas image rendering, shared zoom state, export, and panel routing. This should be split.
- `HistogramPanel.jsx` is also large at ~1100 LOC. Most of that is option/state plumbing and could be moved behind a smaller “chart model -> renderer” boundary.
- There are still frontend `panel_id` special cases for `pdf_adaptation_*` and `gammaloop_*`. Some are legitimate overlays, but most would be cleaner as backend-declared panel groups/views.
- Polling is simple and works, but it is still timer-driven across many hooks. Large finished panels benefit from `poll_after_ms = null`, but general run/task lists still poll frequently.
- UX confirmation uses `window.confirm`, which is functional but inconsistent with the rest of the MUI UI.
- Snackbar logic is duplicated in `App.jsx`, `WorkersWorkspace`, `WorkerDetailsPanel`, and `SettingsWorkspace`.

**High-Value UX Improvements**
- Add collapsible panel sections: `Summary`, `Plots`, `Diagnostics`, `Raw config`. Default to summary plus the most relevant plot. This avoids overwhelming PDF/GammaLoop runs.
- Add panel pinning/favorites per task type. For demos, the operator can pin “central value”, “selected histogram”, or “sampling accuracy”.
- Add a task “focus mode”: hide run config/evaluator/sampler config panels and show only task output. Useful for live demos.
- Replace `window.confirm` with one reusable destructive-action dialog showing the exact run/task/node affected.
- Improve loading states: distinguish “waiting for first output”, “task finished with no panels”, and “loading updated panels”.
- Add keyboard/mouse hints for image panels: “wheel zoom, drag pan, double click reset”. This is needed now that the canvas renderer is custom.

**Make It Lighter**
- Split `PanelCollection.jsx` into `PanelCollection`, `Image2dPanel`, `TimeseriesPanel`, `PdfOverlayPanels`, and `panelComposition.js`. This is the best simplification.
- Move remaining PDF overlay construction to the backend where possible. Frontend should render declared overlays instead of detecting panel pairs by ID.
- Replace the repeated snackbar state with a tiny `SnackbarProvider` or `useSnackbar` hook.
- Use a single `useTemplateList(kind)` hook for run/task/node templates.
- Add `React.lazy` boundaries for heavy panels: `HistogramPanel`, `Image2dPanel`, and maybe `TablePanel`. Right now `PanelCollection` imports most renderers directly.
- Consider replacing ECharts for simple scalar/multi-timeseries with a lighter canvas/SVG renderer later. ECharts is still valuable for histograms, but it is heavy for basic line plots.

**Suggested Next Commit**
Extract `Image2dPanel` from `PanelCollection.jsx` first. It is now self-contained and large enough to justify its own file, and this would immediately reduce the main panel component complexity without changing behavior.
