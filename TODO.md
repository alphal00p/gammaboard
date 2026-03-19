# TODO

## Platform Features
- [ ] add basic auth to dashboard which enables steering (spawning virtual workers, creating runs etc.)
- [ ] implement madnis sampler_aggregator as parametrization.

### Basic Dashboard Auth And Steering
- [ ] keep auth deliberately basic and operator-oriented: one shared dashboard password, no users/roles/sessions table unless requirements change.
- [ ] do not send a pre-hashed password from the browser as the long-lived credential; that just turns the hash into the password-equivalent secret. Prefer sending the raw password once over HTTPS to obtain a short-lived signed admin token or signed cookie.
- [ ] keep read-only dashboard endpoints unauthenticated for now, and gate only mutating endpoints behind admin auth. That keeps the current monitoring UX simple while protecting steering actions.
- [ ] add a single backend auth config like `GAMMABOARD_ADMIN_PASSWORD_HASH` using a standard password hash format (argon2id preferred, bcrypt acceptable if already present). Verify server-side; do not store plaintext.
- [ ] add one minimal `POST /admin/login` endpoint that verifies the password and returns either:
  - an HttpOnly secure cookie, simplest for same-origin dashboard deployments, or
  - a short-lived signed bearer token if cross-origin hosting is required.
- [ ] keep token contents minimal: just `admin=true`, issued-at, expiry. No DB-backed session store unless revocation is explicitly needed.
- [ ] require admin auth on the future mutating API surface:
  - pause/unpause run
  - append/remove/reorder tasks
  - assign/unassign workers
  - create runs
  - possibly node shutdown if exposed in the dashboard
- [ ] keep steering APIs explicit instead of generic patch endpoints. Prefer narrow commands like:
  - `POST /runs/:id/pause`
  - `POST /runs/:id/tasks`
  - `POST /nodes/:id/assign`
  - `POST /nodes/:id/unassign`
  This is simpler to secure and reason about than arbitrary mutable JSON.
- [ ] add a tiny frontend auth state:
  - login form with password field
  - “admin mode” indicator
  - retry on 401 by clearing local auth state
  - hide or disable steering controls unless authenticated
- [ ] keep the first dashboard steering UI very small:
  - pause run
  - append task from JSON/TOML snippet
  - assign/unassign worker
  Expand only after the auth boundary is stable.
- [ ] ensure every mutating action is logged through runtime log persistence with `source=dashboard` or similar so operator actions are audit-visible.
- [ ] if cookie auth is used, add basic CSRF protection for mutating routes. Keep it simple:
  - same-site cookie plus origin check may be enough for the first version
  - only add synchronizer tokens if deployment shape requires it
- [ ] document the intended deployment assumption explicitly: this basic auth model is acceptable only behind HTTPS and for trusted small-team/internal use, not as public multi-user auth.

## Sweep Findings (2026-03-04)

### Performance
- [ ] avoid per-event single-row DB writes in telemetry sink; batch inserts (`INSERT ... VALUES (...)` in chunks or `COPY`) to reduce write amplification.
- [ ] optimize `fetch_completed_batches` contiguous-prefix query (`ROW_NUMBER` over oldest rows each tick); track a per-run cursor/last-consumed batch id to avoid repeated rescans.
- [ ] implement pausing a run by serializing the sampler_aggregator. When pausing a run, we would need to make sure to handle the batch queue

### Duplication Of Code
- [ ] replace repeated polling state machines in `dashboard/src/hooks/useRuns.js`, `useWorkersData.js`, `useTaskOutput.js`, `useRunPerformancePanels.js`, and `useWorkerLogs.js` with a reusable `usePolledResource`-style hook.
- [ ] unify implementation-panel registry dispatch in `dashboard/src/components/evaluator/EvaluatorCustomPanel.jsx` and `dashboard/src/components/sampler/SamplerCustomPanel.jsx`, or fold both into the shared panel infrastructure when the remaining custom implementation views are retired.
- [ ] factor shared control-plane node SQL in `src/stores/queries/control_plane.rs`: repeated `nodes` update clauses for clearing desired assignments and repeated fixed control-plane row defaults.
- [ ] add shared row-decoding helpers in `src/stores/queries/read.rs` for repeated bigint-to-string id conversion, JSON metric parsing, and default-on-decode-failure behavior.

## Sweep Findings (2026-03-18)

### Complexity
- [ ] split `src/server/task_panels.rs` by task kind or adapter role; it still mixes sample-task aggregate decoding, deterministic full-observable rendering, descriptor definitions, and current/history projection in one file.
- [ ] introduce a shared deterministic full-observable task adapter for `image` and `plot_line`; both tasks currently duplicate progress/current-from-persisted/current-from-runtime wiring and only differ in geometry-specific rendering.
- [ ] extract a small aggregate-observable panel builder for sample tasks; scalar/complex estimate, summary, and history projection still live as ad hoc helper sets inside `src/server/task_panels.rs`.
- [ ] replace repeated frontend polling hooks with a shared `usePolledResource` abstraction that covers scheduling, stale-response protection, and `isConnected`/error handling consistently.

## Sweep Findings (2026-03-19)

### Backend
- [ ] reduce duplicated node-assignment SQL in `src/stores/queries/control_plane.rs`; clearing desired assignments, setting current assignments, and timestamp updates still repeat the same `nodes` update clauses and raw-row mapping patterns.
- [ ] add shared decode helpers in `src/stores/queries/read.rs` for repeated JSON-to-typed-metrics conversion, `BIGINT`-to-string id mapping, and typed decode error wrapping; the current row adapters still repeat the same conversion shapes.
- [ ] simplify `src/server/mod.rs`; most handlers are still thin boilerplate around `Path`/`Query` extraction, `store` calls, `NotFound` conversion, and `json_response(...)`, which suggests a small shared pattern for “load one”, “load many”, and “load history”.
