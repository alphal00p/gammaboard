# AGENTS

## Purpose
This file is for contributors making structural or behavioral changes.
Use `README.md` for installation and basic usage. Keep this file focused on architecture, invariants, and implementation rules.

## Module Ownership
- `src/core/*`: cross-stage shared contracts and types, including run-spec/config enums, shared errors, control-plane/store-facing contracts, DB row models, and worker assignment models.
- `src/evaluation/*`: evaluator-side batch/result semantics, evaluator traits, evaluator implementations, and observables.
- `src/sampling/*`: sampler-side latent batch semantics, sampling plans, sampler traits, sampler implementations, and parametrization implementations.
- `src/runners/*`: evaluator, sampler task executors, and node orchestration loops only.
- `src/stores/*`: PostgreSQL implementation, queries, read DTOs, and store composition.
- `src/server/*`: dashboard read API runtime and handlers.
- `src/server/task_panels/*`: task-scoped panel projection helpers; keep `mod.rs` as thin dispatch and split task-family-specific panel logic into focused modules.
- `src/tracing.rs`: tracing setup and DB log sink wiring.
- `src/cli/*` and `src/main.rs`: CLI argument parsing, wiring, and process bootstrap.
- Keep shared CLI store/tracing bootstrap in `src/cli/shared.rs`; avoid repeating `init_pg_store` + `init_tracing` wiring in individual CLI commands.

## Core Invariants
- PostgreSQL is the source of truth for runs, batches, node state, logs, and snapshots.
- Run config is TOML via `gammaboard run add <file.toml>` and is deep-merged over `configs/default.toml`.
- Preprocessing first resolves inherited `RunTaskInputSpec` entries into concrete `RunTaskSpec` stages. Preflight then derives `point_spec` from the evaluator, reduces the resolved task queue via each task's `into_preflight` hook, and executes that reduced queue synchronously in-memory before persistence.
- `run add` persists evaluator and sampler init metadata.
- Runs are driven by persisted `run_tasks`; this is the top-level execution queue and is distinct from the evaluator batch work queue.
- If a run-add config omits `task_queue`, the run is created idle; no batches should be produced until tasks are appended explicitly.
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
- Overall worker reconciliation belongs in `src/runners/node_runner/reconcile.rs`: it should decide the exact active runtime for the current desired assignment, including sampler task activation, pause handling, and queue exhaustion.
- The sampler executor itself should own only one active task runtime and should not activate or select tasks on its own.
- `run-node` should own one in-process active role runner at a time. Reconciliation is the only place that matches on role and constructs a new role runner; active role runners should just expose `tick()` and `persist_state()`.
- Active role runners must not clear node/run assignments themselves. On any unassign path, `run-node` should call `persist_state()` first, then clear assignments and reconcile.
- `NodeRunnerConfig.min_tick_time` is a global pacing guard for the node loop; keep role-runner ticks free of ad hoc sleep/backoff logic unless a hot-loop reason forces it.
- Do not keep stale per-role pacing knobs in run config. Evaluator and sampler runner params should only carry settings still used by the role runtime.

## Engine And Data Rules
- Keep evaluator-side concrete batch/result semantics in `src/evaluation/*` and sampler-side latent queue semantics in `src/sampling/*`.
- Keep observables in `src/evaluation/observable/*` so evaluator-specific observable implementations stay with evaluator implementation families.
- Keep runtime construction config-based through `EvaluatorConfig`, `SamplerAggregatorConfig`, and `ParametrizationConfig`.
- Avoid adding runtime dispatch enums for evaluator/sampler/parametrization selection beyond the config enums.
- Engine config enums should carry typed parameter structs directly; avoid untyped `serde_json::Value` maps at the config boundary.
- `IntegrationParams` and `RunSpec` carry strongly typed runner params.
- Evaluators are batch-oriented and must request training values based on the active task's sampler config, not a per-batch persisted flag.
- Observable semantics are first-class run/task config via `ObservableConfig`, while serialized runtime/current state remains semantic `ObservableState`.
- Queue payloads are latent and task-bound: `batches.latent_batch` plus `batches.task_id`.
- Top-level run task sequencing lives in `src/core/tasks.rs`; sampler/evaluator engines should not parse arbitrary task JSON directly.
- Task-local structural validation and preflight reduction hooks belong on the task types in `src/core/tasks.rs`; `src/preprocess/*` should orchestrate them rather than duplicate task semantics.
- Keep latent-batch queue types separate from concrete evaluator batch/result types in code layout: latent queue payloads belong with sampler-side semantics, while `Batch`/`BatchResult` are the concrete A/B interface.
- `core` owns the cross-stage shared config/run-spec types and error types; concrete evaluator/sampler transport types should still live in `evaluation` or `sampling`.
- Keep the fine-grained store traits in `src/core/traits.rs` for implementation composition, but prefer coarse runner-facing facades like `EvaluatorWorkerStore` and `SamplerWorkerStore` at runner boundaries so worker constructors stay readable.
- Parametrization state lives in task/stage snapshots as `{ config, snapshot }`; evaluators rebuild parametrizations from the activation snapshot resolved through `batches.task_id`.
- Task-owned phase transitions replace sampler-emitted semantic advances: `sample` tasks carry both `sampler_aggregator` and `parametrization` config, while `image` and `plot_line` tasks own deterministic scan geometry and resolve internally to raster sampler + identity parametrization stages. The runner activates the next phase only after the current queue is drained and the current sampler snapshot is persisted.
- Task transitions must restore unspecified runtime state from the latest prior queue-empty `run_stage_snapshots` row, not from transient in-memory handoff only. In particular, sampler snapshot, observable state, and parametrization snapshot handoff should be sourced from persisted stage snapshots.
- Executable tasks may optionally declare `start_from = { run_id, task_id }`. When present, task activation must restore runtime state from the latest queue-empty stage snapshot of that referenced task instead of the default previous-stage lookup.
- `sample` tasks may leave `observable` unspecified. In that case, transition activation reuses the previous observable state as-is; specifying `observable` explicitly starts a fresh observable of that config.
- `image` and `plot_line` tasks must declare their observable family explicitly in task config and always start with a fresh full observable of that family.
- Sample-task config files may omit `sampler_aggregator` and/or `parametrization`; preprocessing must resolve those fields by inheriting the previous effective sample-stage settings before tasks are persisted.
- For task-driven runs, do not duplicate sampler config at the top level of the run-add TOML. Resolve the initial sampler from the first sample task during preprocessing and persist the concrete resolved `integration_params`.
- `Parametrization` currently means full latent-batch-to-concrete-batch materialization. It may later be split into a narrower `Parametrization` plus a `LatentBatchMaterializer`.
- `SamplerAggregatorConfig::HavanaTraining` is the adaptive training sampler phase, and `SamplerAggregatorConfig::HavanaInference` is the compact seed-dispatch phase.
- Havana training budget comes from the active sample task's `nr_samples`, not from sampler config.
- `ParametrizationConfig::HavanaInference` remains declarative; building it may require a persisted Havana training sampler snapshot and/or a persisted previous parametrization snapshot, and the resulting parametrization state must be snapshottable for replay on evaluator workers.
- `LatentBatchPayload::HavanaInference` is the compact evaluator-side Havana inference payload and carries only the per-batch seed; the frozen grid lives in the task activation snapshot's parametrization state.
- If an evaluator supports multiple semantic value families, that choice still belongs in evaluator config via `observable_kind`, but the aggregate-vs-full observable shape is selected explicitly through `ObservableConfig`.
- Evaluator implementations should reuse `IngestScalar` / `IngestComplex` helpers internally after locally matching the resolved observable config; do not reintroduce hidden capture-mode switches in batch/eval options.
- Image and line tasks use full observables that store weighted per-sample values in deterministic task order; use those full observables as the canonical current-state artifact instead of tunneling sample values through training-mode side channels.
- Sampler-aggregator aggregation is `ObservableState` merge, not capability-style ingest.
- Sampler-aggregators own any per-batch training correlation state internally; do not pass runner-managed batch context back into them.
- If a sampler has a finite training budget, training-value capture is a task-level property derived from the active sampler config. The runner must not enqueue extra training-suite samples beyond that boundary.
- The latest full observable state for the active stage is cached on `runs.current_observable`; treat it as runner state, not as a run-global read-model contract.
- Persisted observable history snapshots store each observable's reduced persistent payload, not the full in-memory observable state.
- `run_stage_snapshots` are runtime handoff state only: store typed sampler snapshot, observable state, sampler config, and parametrization state there. Do not duplicate reduced persisted observable payloads in stage snapshots.
- For full observables used by deterministic scan tasks, the reduced persistent payload is just progress (`processed` count). Completed-task rendering should come from `run_stage_snapshots.observable_state`, not from persisted history rows.
- Observable API/read projections should be produced from typed run configuration context such as `RunSpec`, not by passing store read rows or ad-hoc JSON context into observable implementations.
- `LatentBatchPayload::Batch` is the compatibility payload for now and stores the previous compact row-major batch JSON.
- Completed batches are consumed by the sampler-aggregator and deleted.

## Snapshot, Logging, And Read Rules
- Sampler pause/resume snapshots are persisted on `runs.sampler_runner_snapshot`; keep the persisted shape explicit, typed, and task-local.
- Stage-boundary snapshots are also persisted on `run_stage_snapshots`; each row must record `queue_empty` so deterministic resume eligibility is explicit.
- `run_tasks` should persist the effective snapshot origin used at activation time (`spawned_from_run_id`, `spawned_from_task_id`) so branching/debugging remains visible in the CLI and dashboard.
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
- Performance read views should stay narrow: run-level performance is sampler-oriented throughput/queue state, while evaluator timing views are worker-specific.
- Snapshot rows are point-in-time; do not reintroduce window semantics.
- Read APIs should serialize `BIGINT` IDs as strings.
- Run read payloads should expose `runs.point_spec` as `point_spec`.
- Backend observable/output APIs are task-scoped: persisted observable history only has meaning within a single task/stage, and task types own the digest/projection exposed to the frontend.
- Backend visualization payloads should use the generic panel model in `src/server/panels.rs`; task output and performance/history views should share the same panel vocabulary instead of exposing raw backend-specific JSON shapes to the frontend.
- Panel APIs are poll-based and server-owned: clients send an optional last-seen `cursor`, and the backend returns one `PanelResponse` with stable panel specs plus ordered `replace`/`append` updates.
- Panel cursors are opaque backend tokens. They may encode history compaction state in addition to the last seen snapshot id; the frontend must store and resend them unchanged.
- Panels may also declare backend-owned UI state and actions. Frontends should store and resend `panel_state` keyed by `panel_id`, but should not invent panel semantics locally; interactive controls such as selects/toggles/tabs still belong to the backend panel contract.
- Worker/node detail views should also use backend-generated generic panels; keep metric and diagnostics decoding in Rust instead of frontend JSON-shape checks.
- Evaluator and sampler config/summary views should also prefer backend-generated generic panels over frontend implementation-specific card assembly.
- Older task output should be reconstructed from the latest `run_stage_snapshots` row for that task when the active-stage runner state has already moved on.
- Append updates are only valid when the server can prove a panel can be extended incrementally; otherwise it must send a full `replace` update for that panel.

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

## Commit Discipline
- Prefer commit-sized changes and pause between stages.
- When a stage is commit-sized, suggest the commit message as a bare fenced text block only:
  ```text
  your commit message
  ```
  Do not add extra commentary around it.

## Commit Discipline
- Prefer commit-sized changes: finish one coherent stage, verify it, then stop and commit before starting the next stage.
- After each small stage, explicitly check whether it is time to commit again instead of letting unrelated follow-up edits accumulate.
- When a stage is ready, suggest a concrete commit message so the next commit is easy to make.
