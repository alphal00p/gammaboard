# AGENTS

## Purpose
This file is for contributors making structural or behavioral changes.
Use `README.md` for installation and basic usage. Keep this file focused on architecture, invariants, and implementation rules.

## Module Ownership
- `src/core/*`: cross-stage shared contracts and types, including run-spec/config enums, shared errors, control-plane/store-facing contracts, DB row models, and worker assignment models.
- `src/evaluation/*`: evaluator-side batch/result semantics, evaluator traits, evaluator implementations, and observables.
- `src/sampling/*`: sampler-side latent batch semantics, sampling plans, sampler traits, sampler implementations, and parametrization implementations.
- `src/engines/mod.rs`: compatibility re-exports for the runtime domain modules.
- `src/runners/*`: evaluator, sampler-aggregator, and node orchestration loops only.
- `src/stores/*`: PostgreSQL implementation, queries, read DTOs, and store composition.
- `src/server/*`: dashboard read API runtime and handlers.
- `src/tracing.rs`: tracing setup and DB log sink wiring.
- `src/cli/*` and `src/main.rs`: CLI argument parsing, wiring, and process bootstrap.

## Core Invariants
- PostgreSQL is the source of truth for runs, batches, node state, logs, and snapshots.
- Run config is TOML via `gammaboard run add <file.toml>` and is deep-merged over `configs/default.toml`.
- Preflight derives `point_spec` from the evaluator and performs a one-point sampler -> parametrization -> evaluator dry-run before persistence.
- `run add` persists evaluator and sampler init metadata.
- Runs are driven by persisted `run_tasks`; this is the top-level execution queue and is distinct from the evaluator batch work queue.
- If a run-add config omits `task_queue`, synthesize the default queue from `pause_on_samples`: one sample task, plus a pause task when `pause_on_samples` is set.
- `pause_on_samples` still persists to `runs.target_nr_samples` for operator visibility, but task-local sample budgeting now comes from the active `run_tasks` row.
- Point dimensions are canonical in `runs.point_spec`; do not duplicate them outside evaluator config unless the evaluator intrinsically needs them.
- Run lifecycle is derived from control-plane state; do not add a persisted `runs.status` column unless explicitly requested.
- Run pause is implemented by clearing desired assignments so `run-node` reconciles down cleanly.
- A task that changes sampler or parametrization semantics must not activate until the existing batch queue for the run is fully drained.
- If an active run task fails at a transition boundary, persist that as task state `failed` with a reason and clear desired run assignments.

## Node Assignment Rules
- Desired assignment is node-level: each node may have at most one desired role/run assignment at a time, and `node assign` replaces any existing desired assignment on that node.
- `node unassign` clears the node's desired assignment without requiring a role.
- Desired assignments may include many evaluators per run, but at most one sampler-aggregator per run. Enforce that in the database and surface a clear CLI/store error on violation.
- Current assignments may include many evaluators per run, but at most one current sampler-aggregator per run. Enforce that in the database. Recovery from stale current sampler state is manual for now.
- Desired and current node assignments live directly on `nodes`, with `(desired_run_id, desired_role)` and `(active_run_id, active_role)` required to be both null or both set.
- Nodes register and heartbeat through `nodes` even when idle so inventory is visible before any role assignment.
- `run-node` must stop the old role before starting a new one.
- Role start failures are capped per desired target; after the cap is hit, retries stay disabled until desired assignment changes.
- Node shutdown is a one-shot signal read from `nodes.shutdown_requested_at`.

## Engine And Data Rules
- Keep evaluator-side concrete batch/result semantics in `src/evaluation/*` and sampler-side latent queue semantics in `src/sampling/*`.
- Keep observables in `src/evaluation/observable/*` so evaluator-specific observable implementations stay with evaluator implementation families.
- Keep runtime construction config-based through `EvaluatorConfig`, `SamplerAggregatorConfig`, and `ParametrizationConfig`.
- Avoid adding runtime dispatch enums for evaluator/sampler/parametrization selection beyond the config enums.
- Engine config enums should carry typed parameter structs directly; avoid untyped `serde_json::Value` maps at the config boundary.
- `IntegrationParams` and `RunSpec` carry strongly typed runner params.
- Evaluators are batch-oriented and must respect `batches.requires_training`.
- Observable semantics are evaluator-owned and serialized as semantic `ObservableState`.
- Queue payloads are latent and versioned: `batches.latent_batch` plus `batches.parametrization_state_version`.
- Top-level run task sequencing lives in `src/core/tasks.rs`; sampler/evaluator engines should not parse arbitrary task JSON directly.
- Keep latent-batch queue types separate from concrete evaluator batch/result types in code layout: latent queue payloads belong with sampler-side semantics, while `Batch`/`BatchResult` are the concrete A/B interface.
- `core` owns the cross-stage shared config/run-spec types and error types; concrete evaluator/sampler transport types should still live in `evaluation` or `sampling`.
- Parametrization versions are persisted separately in `parametrization_states (run_id, version)` as full `ParametrizationConfig` payloads; evaluators rebuild the parametrization when that version changes, and the version row must be written before any latent batch references it.
- If an evaluator supports multiple observable semantics, that choice belongs in evaluator config via `observable_kind`.
- Sampler-aggregator aggregation is `ObservableState` merge, not capability-style ingest.
- Sampler-aggregators own any per-batch training correlation state internally; do not pass runner-managed batch context back into them.
- If a sampler has a finite training budget, only the exact training-suite samples may be produced with `requires_training`; the runner must not enqueue extra training batches beyond that boundary.
- The latest full aggregated observable lives on `runs.current_observable`.
- Snapshot persistence uses each observable's reduced persistent payload, not the tagged runtime enum form.
- `LatentBatchPayload::Batch` is the compatibility payload for now and stores the previous compact row-major batch JSON.
- Completed batches are consumed by the sampler-aggregator and deleted.
- `ReconfigureSampler` tasks are interpreted by the sampler runner and executed through explicit sampler transition/build APIs; keep task data declarative.
- `ReconfigureParametrization` tasks must persist a new parametrization version before any subsequent latent batch references it.

## Snapshot, Logging, And Read Rules
- Sampler pause/resume snapshots are persisted on `runs.sampler_runner_snapshot`; keep the persisted shape explicit and versioned.
- Adding a new sampler requires snapshot export/restore support for it.
- Do not expose `runs.sampler_runner_snapshot` through the read API or dashboard payloads.
- Runtime logs are persisted from tracing context through `RuntimeLogStore`.
- SQL for runtime log persistence lives in the store/query layer, not in tracing setup.
- Runtime log context should include `source`, `run_id`, and `node_id` when available. Include `worker_id` only when persisted schema still requires that name.
- Set `GAMMABOARD_DISABLE_DB_LOGS=1` to disable DB log persistence.
- DB log thresholds are configured with `GAMMABOARD_DB_LOG_LEVEL` and `GAMMABOARD_DB_EXTERNAL_LOG_LEVEL`.
- Worker performance history is stored in `evaluator_performance_history` and `sampler_aggregator_performance_history`.
- Evaluator performance history stays generic-metrics-only; evaluator-specific diagnostics belong in init metadata.
- Sampler performance history may include generic sampler metrics and sampler runtime metrics, but sampler-specific diagnostics should remain separate.
- Snapshot rows are point-in-time; do not reintroduce window semantics.
- Read APIs should serialize `BIGINT` IDs as strings.
- Run read payloads should expose `runs.point_spec` as `point_spec`.

## Schema Policy
- No backward-compat requirement by default.
- Prefer direct current-schema migrations unless compatibility work is explicitly requested.

## Required Checks
- `cargo fmt`
- `cargo check -q`
- `cargo test -q`
- `just test-e2e` for the ignored full-stack CLI flow against a real local Postgres instance

## Documentation Rule
If you change structure, operations, CLI behavior, config schema, or runtime behavior, update `AGENTS.md` in the same change. If the change affects installation, setup, or normal operator workflow, update `README.md` too.
