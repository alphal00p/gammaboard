# Codebase Review TODO

## P0: Correctness / Operational Risk

- [x] **Clamp aggregated history limit in API**
  - Problem: `GET /runs/:id/aggregated` passes `params.limit` directly while other endpoints clamp bounds.
  - Impact: unbounded/oversized queries can degrade API and DB under load.
  - Ref: `src/bin/server.rs:269-275`.
  - Action: apply `.clamp(1, 10_000)` (or shared validated query type) before store call.

- [x] **Avoid panic on missing `DATABASE_URL`**
  - Problem: startup uses `.expect("DATABASE_URL must be set")`.
  - Impact: hard panic path instead of typed error propagation; poor operability.
  - Ref: `src/lib.rs:46`.
  - Action: map env var read to `sqlx::Error::Configuration` (or project error type) and return `Result`.

- [x] **Remove production `expect("standard order")` in evaluators**
  - Problem: several evaluators panic on non-standard ndarray layout.
  - Impact: avoidable process crash instead of surfaced engine error.
  - Ref: `src/engines/evaluator/unit.rs:48`, `src/engines/evaluator/test_only_sin.rs:87`, `src/engines/evaluator/test_only_sinc.rs:87`.
  - Action: switch to `ok_or_else(...)` and return `EvalError`.

## P1: Performance / Scalability

- [x] **Polling overlap in frontend hooks**
  - Problem: `setInterval(async ...)` can overlap when request latency > interval.
  - Impact: request pile-up, stale race conditions, unnecessary backend load.
  - Ref: `dashboard/src/hooks/useRuns.js:9-30`, `dashboard/src/context/RunHistoryContext.jsx:126-149`.
  - Action: add in-flight guards + `AbortController`; prefer recursive `setTimeout` pattern.

- [x] **Polling + SSE duplication in run history**
  - Problem: context keeps polling queue/logs while also maintaining SSE for stats.
  - Impact: duplicated network/DB load and more state reconciliation complexity.
  - Ref: `dashboard/src/context/RunHistoryContext.jsx:126-192`.
  - Action: split "backfill" vs "live delta" responsibilities and reduce polling when stream is healthy.

- [x] **`run_progress` is a hot-path broad view**
  - Problem: read endpoints and streams repeatedly hit view-based aggregates.
  - Impact: expensive repeated grouping under frequent dashboard refresh/streaming.
  - Ref: read paths through `src/stores/queries/read.rs` and server stream loop in `src/bin/server.rs`.
  - Action: move to incremental counters/materialized per-run summary table updated by runners.

- [x] **One polling loop per SSE client**
  - Problem: SSE endpoint spins per-connection polling loop.
  - Impact: N clients => N DB pollers for same run.
  - Ref: stream implementation in `src/bin/server.rs` (stats stream).
  - Action: introduce per-run fanout task + broadcast channel; clients subscribe to shared stream.

## P1: Maintainability / Simplicity

- [ ] **`just live-test` is heavily hardcoded and brittle**
  - Problem: manual 14 worker starts + fixed run IDs + fixed assignment script.
  - Impact: difficult to change, fails in non-empty DB, high copy/paste overhead.
  - Ref: `justfile:49-104`.
  - Action: move run setup to a small script/helper with generated IDs and declarative topology.

- [ ] **Large low-cohesion files should be split**
  - Problem: single files own many concerns.
  - Impact: harder review, testing, and change isolation.
  - Ref: `src/runners/node_runner.rs` (~823 LOC), `src/stores/pg_store.rs` (~806 LOC), `src/bin/control_plane.rs` (~318 LOC).
  - Action: extract submodules:
    - node runner lifecycle/reconcile/start-stop separation
    - store decode helpers vs trait impl blocks
    - control-plane subcommands per domain (run/assign/worker)

- [ ] **SQL projection duplication for run progress**
  - Problem: repeated long `SELECT` field lists in `get_all_runs` and `get_run_progress`.
  - Impact: drift risk and noisy maintenance.
  - Ref: `src/stores/queries/read.rs:248-312`.
  - Action: extract shared projection constant/query builder.

- [ ] **Dead deprecated hook still shipped**
  - Problem: `useRunData` only throws.
  - Impact: dead code path and possible accidental imports.
  - Ref: `dashboard/src/hooks/useRunData.js:1-3`.
  - Action: remove file and update imports/exports accordingly.

## P2: API / UX Consistency

- [x] **CLI run command UX is inconsistent**
  - Problem: `run-start` takes `--run-id`; `run-pause`/`run-stop` use positional IDs.
  - Impact: avoidable operator confusion and scripting friction.
  - Ref: `src/bin/control_plane.rs` command handling.
  - Action: normalize argument style across run lifecycle commands.

- [x] **`run-add` embeds a large fallback config blob in code**
  - Problem: defaults are inline JSON in command handler.
  - Impact: hard to maintain and easy to diverge from config files/docs.
  - Ref: `src/bin/control_plane.rs:220-255`.
  - Action: move default template to config file or dedicated typed builder.

- [ ] **Frontend API error messages drop response body details**
  - Problem: error helper only uses `statusText`.
  - Impact: hides backend diagnostics and slows debugging.
  - Ref: `dashboard/src/services/api.js:3-6`.
  - Action: parse JSON/text error payloads and include server message in thrown error.

## P2: Future-facing Refinements

- [ ] **Symbolica init metadata should include frontend-ready display fields**
  - Problem: metadata currently returns raw `expr` + `args` only.
  - Impact: frontend must reconstruct display format itself.
  - Ref: `src/engines/evaluator/symbolica.rs:167-171`.
  - Action: add optional rendered fields (for example `latex`) in `get_init_metadata`.

- [ ] **Logs transport should support incremental fetch**
  - Problem: dashboard repeatedly fetches latest fixed-size log window.
  - Impact: repeated transfer of unchanged rows.
  - Ref: `dashboard/src/context/RunHistoryContext.jsx` + log endpoint usage.
  - Action: add cursor (`since_id`) log endpoint and append-only client updates.



# Missing Features

 - [ ] add basic auth to dashboard which enables steering (spawning virtual workers, creating runs etc.)
 - [ ] implement gammaloop evaluator
 - [ ] implement madnis sampler_aggregator
