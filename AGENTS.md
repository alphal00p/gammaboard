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
- `ops/*/config`: runtime/server/deploy profiles. `src/config_defaults`: embedded defaults.

## Model
- PostgreSQL is the source of truth for runs, tasks, batches, nodes, logs, checkpoints, and snapshots.
- Runs are driven by persisted `run_tasks`; snapshots are the branchable state timeline.
- Run names are human-facing and not unique; ambiguous CLI name references must fail.
- Node identity is `name` plus live-process `uuid`; desired/current assignments live on `nodes`.
- Run layout uses `Domain`; concrete evaluator batches are `Vec<Point>`.
- Process evaluators use `kind = "process_evaluator"` and the
  `gammaboard-jsonrpc-v1` `eval_batch` method. Responses use
  `values_row_major`; process evaluator tasks should use a vector accumulator
  with matching components.
- Multi-component observables use vector accumulator models. Full raster
  outputs use `FullVectorAccumulatorState` with named components.

## Design Rules
- Keep adapters thin; put reusable behavior in `src/api` or lower layers.
- Keep `RunSpec` run-global and immutable. Task-varying sampler/materializer/transform/accumulator choices belong on tasks or persisted stage defaults.
- Restore task transitions from persisted snapshots/checkpoints, not in-memory handoff.
- Persist/API payloads must be JSON-safe
- Backend owns panel/read-model semantics; frontend should render generic panel payloads.

## Ops Rules
- `gammaboard deploy run` is the normal dashboard stack supervisor for nginx/backend/Postgres.
- GammaLoop support is controlled by the default `gammaloop` Cargo feature. `--no-default-features` builds must compile without `gammalooprs`/`gammaloop_api` and should return explicit unsupported-feature errors for GammaLoop configs.

## Maintenance
- No backward compatibility by default; prefer direct current-schema migrations unless requested.
- Remove duplication and unnecessary indirection whenever you spot simple cases of it, for more complicated refactors ask the operator.
- Update this file for signnificant architecture/runtime/CLI/config shape changes.
- Update `README.md` for setup/operator workflow changes.
- If Rust code changes: run `cargo fmt`, `cargo check -q`, `cargo test -q`. Run `just test-e2e` only for larger changes touching relevant code.
- After a coherent stage, provide a commit message as a bare fenced `text` block.
