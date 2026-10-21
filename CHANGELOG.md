# Changelog

## 0.3.0 — Unreleased

See the [validation and risk report](docs/validation/2026-10-15-consolidation.md)
for tested workflows, fixes found in the dashboard, and remaining limitations.

- Scaled Voronoi stratification with frozen exact discrete probabilities, zero
  accounting, per-stratum covariance, durable recovery, and task diagnostics.
  [Tetrahedron A/B results](benchmarks/stratification/README.md) include uncertainty
  calibration and the observed finite-sample tail limitation.
- Named real-vector Symbolica expressions and matching train/inference run cards.

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
- Process protocol **v7**: explicit fresh/restore/stage-handoff initialization,
  shared batch types, mandatory sampler restoration, and weighted observable feedback.
- Whole-batch validation rejects invalid samples/overflow without publishing
  partial results. Database disconnects and suspended submissions recover safely.

**Upgrade:** a fresh database and matching workers are required. Old checkpoints
and queue formats cannot be resumed. Export run definitions before resetting;
see [deployment](docs/deployment.md#upgrading-this-revision).
