# AGENTS

Be brief.

Use `README.md` for setup and operator workflows. This file is only for codebase orientation and invariants.

## Code Map
- `src/core`: shared domain types, task/run specs, traits, errors.
- `src/api`: typed use cases shared by CLI and server.
- `src/stores`: PostgreSQL queries/read models.
- `src/evaluation`, `src/sampling`, `src/runners`: engine semantics, queues, workers.
- `src/process_runtime`, `src/process_worker`: process command launchers and framed JSON-RPC worker protocol.
- `src/server`: API and backend-owned panel models.
- `src/cli`: command parsing/bootstrap.
- `integrations/*`: optional external-tool wrappers and heavier examples;
  keep these out of the default local `resources/` space.
- `ops/*/config`: runtime/server profiles. `src/config_defaults`: embedded defaults.

## Model
- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, checkpoints, and snapshots.
- Runs are driven by persisted `run_tasks`; snapshots are the branchable state timeline.
- Run names are human-facing and not unique; ambiguous CLI name references must fail.
- Node identity is `name` plus live-process `uuid`; desired/current assignments live on `nodes`.
- Run layout uses `Domain`; concrete evaluator batches are `Vec<Point>`.
- Evaluator runners validate concrete materialized/transformed batches against
  the run `Domain` before calling the evaluator.
- `Domain::Rectangular` is a compact first-class domain for fixed-cardinality
  discrete grids with fixed continuous dimensionality; do not expand it into a branch tree.
- Raster geometry uses its `discrete` path to select a domain branch with a
  unique continuous dimensionality; it does not require globally rectangular domains.
- Process evaluators use `kind = "process_evaluator"` and the
  `gammaboard-jsonrpc-v1` `eval_batch` method. Requests use ragged row-major
  coordinate arrays plus offsets; responses use `values_row_major`. Process
  evaluator tasks should use a vector accumulator with matching components.
- Process samplers may expose `discrete_pdf` for batched marginal PDFs over
  discrete subspaces. The live sampler runner probes it during performance
  snapshots and exposes the latest values through sampler diagnostics; panel
  projection must not rebuild large samplers to fetch these values.
- Multi-component observables use vector accumulator models. Full raster
  outputs use `FullVectorAccumulatorState` with named components.
- Symbolica evaluators use either a native expression plus optional `constants`
  or a precompiled Symbolica shared library. `args` are sampled coordinates only;
  compiled evaluators may declare `compiled_args` plus fixed `bindings` for
  non-sampled inputs. `i`/`I` are built-in imaginary-unit constants. Purely real
  results work with scalar accumulators, while complex results require
  vector/full-vector components `real` and `imag`.
- `image2d` panels are rendered by the generic canvas heatmap path. Complex
  image tasks can use `display = "complex_phase"` to draw phase as hue and
  magnitude as saturation.
- Scalar/vector accumulators may opt into higher moments with
  `moments = { max_order = 4 }`. Generic accumulator metric extraction returns
  JSON-safe metric values plus optional uncertainty; sample stop conditions may
  target these metrics.
- Sample tasks may declare a task-local `measurement` with `quantity` and
  `mode`. Stopping stays on the sample `stop_condition`; external measurement
  references only add `source_task` around the same source-less measurement
  fields.
- `parameter_scan` is a control-plane controller task. It spawns normal grouped
  child runs from `trial_run_toml`, stores scan progress in
  `run_tasks.controller_output`, and reads child task `measurement_output`.
  It supports finite Cartesian scans via `[[parameters]]`; legacy single
  `[parameter]` configs are normalized to the same internal shape.
- `hyperparameter_tuning` is also a control-plane controller task. It owns
  trial child-run lifecycle and measurement collection; optimizer algorithms
  only plan parameter candidates. `optimizer.algorithm` selects the adapter and
  all algorithm-specific knobs, including seeds and budgets, live in
  `optimizer.params`.
- Run, task-append, and node-launch TOML may use a top-level `replacements`
  table plus placeholders `$(name:default)`. Exact full-string placeholders are
  typed TOML replacements; embedded placeholders interpolate as strings. Server
  and runtime configs are not templated.
- Evaluator config is stage state. The run-global evaluator is optional and,
  when present, is the root/default; sample/image/plot-line tasks may set `evaluator = "latest"`,
  `{ from_name = "..." }`, or `{ config = ... }`. Task-level evaluators must
  resolve to the run root domain.
- Evaluators may expose JSON-safe metadata after initialization. Sampler
  activation instantiates the effective evaluator first, reads this metadata,
  and passes it into sampler construction; process samplers receive it as
  `evaluator_metadata` in `initialize`.

## Design Rules
- Keep adapters thin; put reusable behavior in `src/api` or lower layers.
- Keep `RunSpec` run-global and immutable. Task-varying sampler/materializer/transform/accumulator choices belong on tasks or persisted stage defaults.
- Restore task transitions from persisted snapshots/checkpoints, not in-memory handoff.
- Run task activation and controller tasks are control-plane work. The node
  supervisor leader advances tasks and runs controller tasks without consuming a
  sampler/evaluator assignment; sampler workers only execute already-active
  compute tasks.
- Persist/API payloads must be JSON-safe
- Backend owns panel/read-model semantics; frontend should render generic panel payloads.

## Ops Rules
- `gammaboard deploy run` is the normal dashboard stack supervisor for nginx/backend/Postgres.
- `server.toml` owns both backend API settings and deploy exposure/cleanup settings; there is no separate deploy config.
- `server.toml` also owns the human-facing server `name` shown by the frontend connection status and settings view.
- `--port-offset` is a global runtime/deploy option: it shifts frontend/API/Postgres ports and local Postgres state paths for every spawned child process.
- GammaLoop support is controlled by the default `gammaloop` Cargo feature. `--no-default-features` builds must compile without `gammalooprs`/`gammaloop_api` and should return explicit unsupported-feature errors for GammaLoop configs.

## Maintenance
- No backward compatibility by default; prefer direct current-schema migrations unless requested.
- Remove duplication and unnecessary indirection whenever you spot simple cases of it, for more complicated refactors ask the operator.
- Update this file for signnificant architecture/runtime/CLI/config shape changes.
- Update `README.md` for setup/operator workflow changes.
- If Rust code changes: run `cargo fmt`, `cargo check -q`, `cargo test -q`. Run `just test-e2e` only for larger changes touching relevant code.
- After a coherent stage, provide a commit message as a bare fenced `text` block.
