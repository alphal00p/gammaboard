# Consolidation validation and risk assessment — 2026-10-15

The review covers the uncommitted consolidation originally based on `f2a3911`,
the 0.3 compatibility work, and fixes found while using the deployed dashboard.
The initial consolidation is committed as `f4e7a21`; subsequent commits preserve
editor drafts (`6593e26`, `452147c`), local launches after executable replacement
(`6556e05`), and atomic stage publication (`c46de01`). The companion
glNIS adapter is committed through `f711db7`.

The result is suitable for controlled deployments with a fresh database and
matching binaries/SDKs. It is not an in-place upgrade of an existing deployment.
Production confidence remains limited by the campaign budget behavior below,
unvalidated native mapping combinations, and the absence of a long-duration,
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
- A separate-output definition converter and an [upgrade guide](../upgrading-0.3.md)
  describe the breaking changes. Checkpoints and numerical results are not migrated.

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

## Remaining behavior and risk

**Campaign budgets are not hard caps.** A valid 512-sample campaign produced
4,096 samples in one fast child before observation, then failed because the other
child had no usable result. With a 16,384 threshold, the same signed campaign
completed correctly. Allocation windows govern scheduling decisions, not sample
reservations. This behavior is now documented in [configuration](../config.md#controllers).
Strict resource caps require reservation/in-flight accounting or explicit child
batch/budget constraints. The post-progress check also exists in the baseline;
the baseline binary was not separately run for this case.

Other observed surprises:

- Completed or failed task queues can show run lifecycle **paused**. Live idle
  workers show **inactive**. Those labels describe scheduling, not success/death.
- Task lists, summaries, and plots poll independently and can briefly disagree.
  The cross-stage publication defect above is fixed; ordinary display lag remains.
- Scalar shorthand is displayed as a vector accumulator, and a selected task's
  sample count can differ from the cumulative run total.
- Canvas-backed SVG exports contain an embedded PNG. They are not editable vector
  plots; JSON export retains the data for independent rendering.
- In passwordless mode, logout only makes the client view read-only; it does not
  revoke server authorization. The login dialog still requires a nonempty password
  field even though authentication is disabled. This remains a usability issue.
- The zero-error signal-to-noise-style diagnostic displays 0 rather than infinity;
  an empty reporting interval briefly displayed negative zero nanoseconds. These
  are diagnostic presentation edge cases, not changes to the stored integral.
- Compute/I/O busy percentages are occupied wall time, can overlap, and are not
  CPU utilization. Process RSS excludes external Python workers and GPUs.
- Settings displayed derived allowed origins twice; this was cosmetic.
- Failed launch requests remain visible as audit history. Forwarded browser origins
  must be explicitly allowed; the two observed Codex localhost origins were added
  and verified to return HTTP 200.

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
controlled throughput/memory comparison, and a longer recovery soak. If a strict
campaign cap matters, address its reservation semantics before relying on it.

Local evidence, screenshots, exported figures, installed wheels, configurations,
and browser action records are under
`/common/dev/cedric/setup-logs/consolidation-release-20261015/`.
