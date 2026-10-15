# Consolidation validation and risk assessment — 2026-10-15

This is a dated validation record. Current behavior and interfaces are specified
in the [execution model](../execution-model.md) and [compatibility policy](../compatibility.md).

The review covers the uncommitted consolidation originally based on `f2a3911`,
the 0.3 compatibility work, and fixes found while using the deployed dashboard.
The initial consolidation is committed as `f4e7a21`; subsequent commits preserve
editor drafts (`6593e26`, `452147c`), local launches after executable replacement
(`6556e05`), and atomic stage publication (`c46de01`). The companion
glNIS adapter is committed through `f711db7`.

The result is suitable for controlled deployments with a fresh database and
matching binaries/SDKs. It is not an in-place upgrade of an existing deployment.
The campaign budget defect and UI findings were addressed in the follow-up below.
Production confidence remains limited by unvalidated native mapping combinations
and the absence of a long-duration,
large-scale performance/recovery comparison.

## What changed

- Scalar configuration normalizes to a statistical vector. Projection statistics,
  measurements, controller child results, and worker reporting have common models.
- Sampling uses a common pipeline configuration and versioned saved sampler state;
  evaluator work carries its resolved accumulator configuration.
- Symbolica has one tagged source; task/queue parsing rejects unknown fields.
  Defaults from a different evaluator kind no longer leak into an override.
- Process protocol v6 and SDK 0.3 require explicit output layouts, ordered weighted
  feedback, and sampler restoration. MadGraph/MadNIS dependencies select this SDK.
- Database, checkpoint, and queue compatibility checks reject unsupported formats.
  Performance responses use schema 2. Historical migrations were replaced by a
  fresh baseline, and operator documentation was consolidated.
- The initial consolidation included a definition converter. The later cleanup
  removed it; [current compatibility policy](../compatibility.md) requires current
  definitions and matching state formats. Checkpoints/results are not migrated.

## Validation

These are local results; remote GitHub Actions was not run for the new commits.
The principal environment was Linux/Nix, Rust 1.99, PostgreSQL 17.9, Node 24,
and Python 3.13. CI uses PostgreSQL 16, so this was not an exact CI image replay.

| Check | Result |
| --- | --- |
| Default Rust library / CLI / ordinary backend contracts / doctest | 355 / 8 / 3 / 1 passed |
| Feature-minimal Rust suite | 344 library tests plus CLI/contracts/doctest passed; final build check passed |
| Strict Clippy, formatting, diff whitespace | Passed |
| PostgreSQL contracts | 41 passed, including schema rejection, abandoned transactions, and stage publication |
| Workflows and process protocol | 31 workflows and 2 process tests passed from a clean checkout; all 32 workflows and both process tests passed after the stage-publication fix |
| Delayed-inference campaign regression | Passed; inference result is used instead of the previous training result |
| Physics integrations | 5 passed, with explicit state and Python-runtime prerequisites |
| Frontend | 144 tests, lint, and build passed; clean dependency install/audit passed with no production dependency vulnerabilities |
| Python SDK / MadGraph wrapper | 35 passed, 1 optional real-MadGraph-state test skipped |
| Benchmark contracts / converter / UBELIX helpers | 43 / 3 / 7 passed |
| Standalone Rust process evaluator | 2 passed, including ragged batches |
| glNIS adapter | 4 contracts passed; installed wheel passed native import, real CPU training, exact next-draw restoration with pending feedback, and inference |

A clean, locked glNIS checkout produced its CPython 3.13/Linux 0.2 wheel. The
installed wheel was tested with SDK 0.3 and the existing scientific dependency
stack. MadNIS and momtrop were pinned to the revisions used for those checks.
Existing local Cargo path overrides invalidate the sibling working tree's lock;
they were preserved, and the clean checkout was used for release validation.
Legacy Python environments retain their original protocol-v2 SDK. A separate
v6 runtime was used for the updated adapter.

## Real-browser exercise

Mutating actions were performed through the dashboard. Read-only HTTP/SQL checks
verified final task states and numerical results. The private deployment uses
frontend port 8085, API port 4015, and PostgreSQL port 5415. Existing deployment
data was not reset. Six local workers were launched/restored, and one was stopped
and replaced through the UI.

| Area | Exercised |
| --- | --- |
| Definitions | Template selection/save/delete, invalid input with draft retention, copy/download, current-definition export, fresh duplication and deletion |
| Tasks | Pending edit, duplicate, delete, append, and stage selection |
| Workers | Local requests, assignment/resume, pause, unassign-all, removing an evaluator, individual shutdown/replacement, restart with saved-worker restoration |
| Tuning | Live draw/batch changes, rejection of inconsistent limits without publishing them, clearing overrides |
| Performance | Overview, diagnostics, graphs, 30 s / 5 min / all windows, drag-selected custom window shared across tabs, JSON/SVG export |
| Sampling | Unit smoke, Havana training/inference, transformed disk integral, and installed glNIS process-sampler training/inference |
| Controllers | Parameter scan, grid search, signed-coefficient campaign, managed-child restrictions and return-to-parent navigation |
| Plots | Image, line, PDF-adaptation image/line, color limits/scales, histogram PDF/ratio/error controls |
| Operations | Log search/empty search/pagination, settings, logout/login; destructive reset/shutdown confirmations were inspected and cancelled |

Selected numerical checks:

- Unit smoke: 8,000 samples followed by a fresh 512-sample task; both means exactly 1.
- Havana inference: `0.5002405 ± 0.0002027` against 0.5, with 400,000 samples.
- Unit-ball transform: `3.1472356 ± 0.0090911` against π, with 40,000 samples.
- Scan of `amplitude*x`: results near 0, 0.5, and 1; grid search selected amplitude 0.
- Signed campaign: `1 - 0.5*1 = 0.5`, zero uncertainty, 8,192 child samples.
- Polynomial/Havana example: `4.1805881 ± 0.0273772` against 25/6; all four plotting tasks completed.
- glNIS process adapter: 64 training samples, then a fresh 1,024-sample inference task, completed through the UI.

These small, mostly single-seed checks demonstrate operation and catch gross
normalization mistakes; they do not establish general statistical calibration.

## Defects found and fixed

1. **Delayed navigation discarded drafts.** A new run appearing in a later poll
   switched back to Runs and closed a Management draft. Navigation now occurs at
   the explicit action; polling only completes selection. A second race closed a
   new-run draft when deletion/polling changed the selected run. Independent drafts
   now survive that change. Both have regression tests; the first was rechecked
   in the live browser.
2. **Rebuilding a running executable broke worker launch.** Linux exposed the
   executable's old path as deleted, causing `ENOENT`. Immediate self-spawns now
   use the running image on Linux, retaining server/worker version coherence.
   A real-process test unlinks the server's temporary executable and verifies
   worker launch and shutdown. The deployment also uses an immutable binary path.
3. **New stages could expose old live results.** Activation could mark inference
   active while `runs.current_observable` still contained training data. This was
   visible in the dashboard and could affect controller result selection. A new
   sampling/plotting stage now clears the old live accumulator atomically with
   activation, using run-before-task lock ordering. Durable history remains intact.
   A database regression failed before the fix; a delayed-inference campaign test
   verifies that training's constant-one result cannot replace inference's 0.5.
4. **Packaging/tests concealed problems.** MadGraph pinned a protocol-v2 SDK while
   CI bypassed dependency resolution. An ignored local `build/` directory also
   produced a new-version wheel with stale exports. Dependency constraints, clean
   installation checks, and required-export checks address those gaps. The
   dashboard export test's deferred URL cleanup was also fixed.
5. **Recovery evidence upload failed.** Server log filenames now replace colons,
   which GitHub artifact upload rejects.

## Follow-up: campaign cap and UI findings

The original 512-sample campaign consuming 4,096 samples was a budget-enforcement
bug. It is fixed by durable per-child reservations acquired before generation.
The sum of reservations cannot exceed the campaign cap, even with concurrent
controllers. A reservation includes completed, queued, and buffered samples across
all child stages. Recovery reuses logical sample slots; evaluator retries can
still execute a slot more than once. This is a sample cap, not a CPU-time cap.

Pilot grants are bounded independently of oversized scheduling windows and child
batch settings. Selection accounts for available samplers, so a lone sampler
rotates through pilots. Unused credits are reclaimed only after a child queue
completes. A campaign waits for final-stage progress publication before declaring
success. A genuinely insufficient cap fails explicitly without overshooting.
The additive migration is `202610150001_campaign_budgets.sql`; deploy matching
controllers and workers. This cannot undo work an older deployment already ran.

The UI findings are addressed as follows:

- Completed and failed queues display those outcomes; live unassigned workers
  display **idle**. Worker placement and queue results remain separate concepts.
- Summaries and selected outputs refresh on task state changes and display update
  times. Aborted requests cannot replace newer data. Live sections still use
  separate requests, so their timestamps describe the observed state.
- Scalar shorthand displays **scalar (one vector component)**. Labels distinguish
  accumulator samples, samples across this run's tasks, and totals with children.
- ECharts SVG exports render actual vector paths, including stacked histogram
  panels. Raster heatmaps export **PNG**. JSON remains available for the data.
- Passwordless deployments offer **Switch to read-only** / **Enable controls**;
  the browser preference survives reload. There is no password prompt or claim
  of server-side logout in this mode. Password-protected login remains unchanged.
- A positive signal with zero error displays **∞**; zero divided by zero displays
  **undefined**. Negative zero is normalized in numeric presentation.
- Activity labels identify wall time; RSS labels explicitly exclude Python/GPU
  memory. Compute and I/O can overlap.
- Settings deduplicate allowed origins. Failed launches appear in collapsed audit
  history with completed/canceled attempts, rather than the outstanding queue.

Follow-up validation includes the original cap scenario with one and two samplers,
a scheduling window larger than the cap, concurrent database grants, replay,
unused-credit reclamation, prior-stage accounting, and final-stage publication.
Final follow-up checks: 357 Rust library tests, 42 PostgreSQL contracts, 40 end-to-end
tests (across the broad run and targeted reruns), 149 frontend tests, strict Clippy,
frontend lint/build, and formatting passed. Scientific adapters were rerun with
their existing Python runtimes and reference state explicitly configured. The
stage-progression test permits early convergence after all final pilots; it still
checks training-work accounting and the final result sources.

The private deployment runs code revision `72f518a` (main fix `d5d03a7`) and restores
six workers. Chromium created and steered these fresh campaigns through the UI:

| Run | Outcome | Produced / completed / reserved | Result |
| --- | --- | --- | --- |
| `ui-budget-fixed` (#23), original 512 cap and error target | Completed | 64 / 64 / 64 | 0.5 ± 0 |
| `ui-budget-hard-cap` (#26), 512 cap and 4,096 allocation window | Completed | 512 / 512 / 512 | 0.5 ± 0 |
| `ui-budget-insufficient` (#29), cap 1 | Explicit budget failure | 1 / 1 / 1 | No combined result |

Read-only SQL verified the counts and reservations. Browser checks also verified
passwordless controls across reload, completed/failed/idle labels, scalar and
sample-scope labels, the infinite zero-error ratio, unique allowed origins, launch
history, and real SVG/PNG downloads. SVG files contain paths and no embedded PNG.
Live sections remain separately timestamped rather than claiming a single atomic
snapshot. JSON exports and existing numerical results are unchanged.

Browser evidence and final command logs use the `budget-` prefix in the artifact
directory below; downloaded figures are in `budget-downloads/`.

## What the tests do not establish

- **Upgrade safety:** no old database/checkpoint conversion or rolling mixed-version
  deployment is supported. New code rejects old formats; an old binary cannot know
  the new compatibility guard. Preserve old software with old backups.
- **Full scientific coverage:** high-dimensional/ragged production inputs, rare
  non-finite tails, severe cancellation, all observable/projection combinations,
  and correlated campaign estimates were not exhaustively validated.
- **Native mapping compatibility:** GammaBoard locks GammaLoop `40616e5`; the glNIS
  native wheel locks `6c3b1ff`. The glNIS UI/CPU checks used a unit evaluator. They
  do not validate its native momentum mappings against newly generated production
  GammaLoop states. Align and test those states before a real GL26/NLO workload.
- **Scale and durability:** no multi-day soak, large database/large histogram stress,
  disk/power-loss test, distributed PostgreSQL failover, or production network
  partition campaign was performed. External sampler artifacts must remain immutable
  and available. Process callbacks can be retried and must tolerate replay.
- **Performance:** existing benchmark reports predate this change. Contract tests
  and dashboard measurements are not a controlled old/new throughput comparison;
  the host was shared and busy. Large-state validation/serialization costs need a
  representative benchmark before production promotion.
- **Platforms and external systems:** GPU execution, Slurm/UBELIX submission,
  Apptainer image builds, a fresh real MadGraph state, and other OS/architecture
  combinations were not exercised. UBELIX helper tests and external-launch queue
  contracts do not establish cluster operation.

The next production gate should be matched native physics-state validation,
controlled throughput/memory comparison, and a longer recovery soak. The campaign cap defect is fixed; the remaining gates are coverage requirements,
not demonstrated defects. Local throughput and recovery studies can be run here;
representative physics states, GPU hardware and cluster access are needed for the
corresponding production checks.

Local evidence, screenshots, exported figures, installed wheels, configurations,
and browser action records are under
`/common/dev/cedric/setup-logs/consolidation-release-20261015/`.
