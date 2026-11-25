# Changelog

## Unreleased — consistent request-oriented CLI

- Worker launch/start/restore submit durable requests using instance policy; a service-owned controller fulfills local requests.
- Added uniform optional launch waiting and queued-request inspection/wait/cancel; cancellation is fenced against controller claims.
- Separated offline administration/probes (`gb local`) from API management, and removed management forwarding from the service executable.
- Made restart whole-instance only; corrected connection/credential precedence, offline config validation, JSON error handling, and HTTP wait deadlines.

## Unreleased — unified control service

- Combined lifecycle coordination, API hosting, and optional dashboard serving in one process and port.
- Added independently buildable `gb` HTTP client; management no longer needs direct PostgreSQL access.
- Kept worker/database communication unchanged; Slurm management and cleanup use HTTP exclusively.
- Split local result downloads from durable server exports (`--export-path`).
- Removed the Unix control socket, separate API child, and standard nginx deployment stack.

## Externally managed instances and reusable Slurm helper

- Add `externally_managed` and per-invocation `--allow-local-processes` gates,
  including aliases, worker execution/restoration, probes, and dashboard launches.
- Move ordinary run control, deferred exports, admission and graceful shutdown to
  PostgreSQL so clients do not require a local supervisor socket.
- Rename the reusable helper to `ops/slurm/slurm.py`, add explicit helper/instance
  configuration and frozen environment files, and retain the UBELIX compatibility
  entry point. Launch-request fetching remains HTTP-based pending the next pass.


## Local CLI consolidation

- Add file-selected local instances, independent database/web service starts,
  explicit additive worker launches, graceful stop/wait, and worker restoration.
- Add create-and-start runs with durable atomic JSON result exports, result retrieval,
  configuration/log/template commands, and logical database maintenance.
- Preserve exact floating-point values across JSON transport so frozen stratification
  plans retain their identities during distributed tetrahedron inference.
- Preserve legacy deployment and Slurm workflows for a separate HPC consolidation.


## 0.3.0 — Unreleased

See the [validation and risk report](docs/validation/2026-10-15-consolidation.md)
for tested workflows, fixes found in the dashboard, and remaining limitations.

- Consolidate the dashboard catalog under `resources/templates/`, with consistent
  tetrahedron run names and one portable replica campaign shared with UBELIX.
  Move physics preparation to `tools/prepare-gammaloop/` and the 1D A/B driver to
  `benchmarks/stratification/`. Retain historical results; remove redundant task
  cards and specialist physics examples. Process tutorials now include a small
  NumPy sampler with recovery and exact PDF/mass queries.
- Remove the bundled MadGraph/MadSpace and MADNIS adapters, environments and
  example cards. Existing commands pointing into `integrations/` must use an
  independently installed adapter. The generic process API remains supported.
- Optional native backends: minimal builds omit Symbolica; `symbolica` enables
  expression evaluation and Havana, and the default `gammaloop` feature includes
  both. Shared domains, stratification and process interfaces remain available.
  See [build features](docs/deployment.md#build-features).
- Concurrent campaigns reserve one sampler and a configurable minimum evaluator
  pool per active child, schedule independent durable sample quanta, and preserve
  other children's workers during switching. Error-only campaigns use the same
  recovery-safe grants; convergence drains accepted work before final publication.
- Scaled Voronoi stratification with frozen exact discrete probabilities, zero
  accounting, per-stratum covariance, durable recovery, and task diagnostics.
  [Tetrahedron A/B results](benchmarks/stratification/README.md) include uncertainty
  calibration and the observed finite-sample tail limitation.
- Named real-vector Symbolica expressions and matching train/inference run cards.
- Hybrid GammaLoop stratification: conditional primary integral moments alongside
  masked, full-weight pooled histograms. Native and process adapters share the
  estimator path.
- Left-bound stratum prefixes support rectangular, ragged and sparse domains.
  Typed Python domains and a shared coordinate codec cover all process roles;
  rich feedback retains unmasked values and requested/actual prefixes while
  exposing masked values to generic samplers.
- Explicit pooled training statistics and separate stratified/basic inference
  estimates, including uncertainty availability, extrema and export metadata.
- Per-stratum contributions, allocation, hit rates and variance shares in the
  dashboard. Ordinary and stratified PDF diagnostics share density, mismatch,
  distribution, coverage and membership views, with exact native Havana bins,
  paged comparisons, linked navigation and exports.
- Stratified uncertainty stays unavailable across measurements, campaigns and UI
  until coverage is adequate; crash/replay and transformed-domain tests verify the
  end-to-end contract. Uniform sampling now includes discrete selection weights.

- Strict task/queue configuration, one tagged Symbolica source, common sampling
  pipeline configuration, and shared stopping estimates in the runner/dashboard.
- Explicit database, checkpoint, and queue compatibility checks; performance
  schema 2 and coordinated SDK/integration dependency versions.
- A [complete execution specification](docs/execution-model.md) and strict current-format
  interfaces. Retired conversion tooling and saved-view adapters were removed.

- Distributed integration with persistent runs, worker pools, live results,
  performance reporting, and allocated core-hour accounting.
- Parameter scans, hyperparameter tuning, and campaigns combining independent
  child estimates and compatible histograms.
- Native GammaLoop and custom process evaluators/samplers.
- Shared local, ITPhlies, and UBELIX workflows; backup/restore and worker recreation.
- Consolidated accumulator, measurement, sampler-state, and extension interfaces.
  Scalar configuration remains shorthand for a one-component statistical vector.
- Process protocol **v9**: explicit fresh/restore/stage-handoff initialization,
  shared batch types, mandatory sampler restoration and rich weighted feedback.
  Normalized joint PDFs and optionally unspecified discrete masses replace the
  separate global PDF normalization callback; stratum-prefix masses must be exact.
- Whole-batch validation rejects invalid samples/overflow without publishing
  partial results. Database disconnects and suspended submissions recover safely.

**Upgrade:** protocol v9, database model 7, checkpoint model 5 and queue envelope 4
require a fresh database and matching workers. Old checkpoints and queue formats
cannot be resumed. Export run definitions before resetting; see
[compatibility](docs/compatibility.md) and
[upgrade procedure](docs/compatibility.md#upgrade-procedure).
