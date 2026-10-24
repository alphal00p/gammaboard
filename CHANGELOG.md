# Changelog

## 0.3.0 — Unreleased

See the [validation and risk report](docs/validation/2026-10-15-consolidation.md)
for tested workflows, fixes found in the dashboard, and remaining limitations.

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
- GammaLoop, MadGraph, MadNIS, and custom process evaluators/samplers.
- Shared local, ITPhlies, and UBELIX workflows; backup/restore and worker recreation.
- Consolidated accumulator, measurement, sampler-state, and extension interfaces.
  Scalar configuration remains shorthand for a one-component statistical vector.
- Process protocol **v9**: explicit fresh/restore/stage-handoff initialization,
  shared batch types, mandatory sampler restoration and rich weighted feedback.
  Normalized joint PDFs and optionally unspecified discrete masses replace the
  separate global PDF normalization callback; stratum-prefix masses must be exact.
- Whole-batch validation rejects invalid samples/overflow without publishing
  partial results. Database disconnects and suspended submissions recover safely.

**Upgrade:** protocol v9, database model 6, checkpoint model 5 and queue envelope 4
require a fresh database and matching workers. Old checkpoints and queue formats
cannot be resumed. Export run definitions before resetting; see
[compatibility](docs/compatibility.md) and
[deployment](docs/deployment.md#upgrading-this-revision).
