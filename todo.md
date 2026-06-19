# TODO

## Structural Cleanup

1. Split `src/server/mod.rs`.
   - Move route handlers, route wiring, process spawning, DB control, and tests
     into focused modules.
   - This also resolves the current `items_after_test_module` clippy warning.

2. Introduce request/context structs for large argument lists.
   - `EvaluatorRunnerConfig`
   - `SamplerRunnerConfig`
   - `SubmitResult`
   - `RuntimeLogQuery`
   - Use these to address `too_many_arguments` in runners, stores, and server
     APIs.

3. Reduce `RunTaskSpec` enum size.
   - Box or factor large task-specific fields, especially the `Sample` variant.
   - Keep TOML/API shape stable unless there is a clear schema simplification.

4. Extract shared controller-run orchestration.
   - `parameter_scan` and `hyperparameter_tuning` duplicate child-run
     lifecycle logic.
   - Create a shared helper for listing children, classifying measurements,
     spawning capacity, redistributing workers, and persisting progress.

5. Clean server control-handler error logging.
   - Replace repeated `.map_err(|err| { log_control_api_error(...); err })`
     with a small helper or `.inspect_err(...)`.

6. Decide clippy policy.
   - Add `cargo clippy` to regular verification once structural lints are
     either fixed or explicitly allowed.

## Config And Template UX

1. Add template metadata.
   - Required tools/artifacts.
   - Expected runtime.
   - GPU requirement.
   - Demo category.

2. Add run-template preflight.
   - Detect missing `.venv`, SIF images, MADNIS runtime, GammaLoop state paths,
     and missing external tools before run creation.
   - Return actionable messages with suggested setup/build commands.

3. Add a demo-readiness panel.
   - Show `ready`, `missing dependency`, or `unsupported on this host`.
   - Include next commands where possible.

4. Separate quick demos from external-runtime demos.
   - Keep default templates runnable with minimal setup.
   - Put Apptainer/MADNIS/GammaLoop/process-runtime examples in clearly marked
     advanced/demo groups.

## Scan And Tuning UX

1. Improve parent/child run display.
   - Group scan/tuning child runs under the parent by default.
   - Persist collapse state in the frontend.
   - Keep direct child-run navigation from tables.

2. Add compact parent summaries.
   - Best/current child.
   - Completed/running/failed count.
   - Best objective or selected measurement.

3. Add scan/tuning table exports.
   - CSV export.
   - JSON export.
   - Include child run ids and parameter columns.

4. Improve failed-trial visibility.
   - Show concise failure reason in parent panel.
   - Add a direct link to child logs.

## Future Features

1. Trial persistence for adaptive optimizers.
   - Current controller output is compact and works, but a dedicated trial table
     would make restart recovery, querying, and partial failed observations
     more robust.

2. More optimizer policies.
   - Failure policy: fail-fast, tolerate N failures, or penalize failures.
   - Replicates per parameter point.
   - Noisy-objective aggregation.

3. Better scan visualizations.
   - Heatmaps for 2D scans are present; extend to selected metrics/components.
   - Add vector/histogram-over-parameter views later.

4. Artifact generation helpers.
   - `just` targets to build/check process-runtime artifacts used by templates.
   - Optional “start all lightweight examples” command for demos.
