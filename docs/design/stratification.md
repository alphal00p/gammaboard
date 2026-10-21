# Stratification design decisions

Scaled Voronoi stratification is an integration sampling-pipeline feature. It
composes with campaigns/scans/tuning through their child integrations. The current
interface and complete estimator contract are in the
[execution specification](../execution-model.md#scaled-voronoi-stratification).

- Keep full integration weights on generated samples; query exact discrete
  probabilities once for frozen inference and derive conditional weights internally.
- Store the partition in sampling configuration, inherit it with trained proposals,
  and separate its outer label from the evaluator's physical coordinates.
- Gate values before accumulation; retain attempted counts and zero feedback for
  misses. Native aggregate observable bundles need adapter-level support first.
- Use an explicit fresh accumulator for inference. Immutable proposal identity,
  probabilities, and per-stratum moments survive splitting and checkpoint recovery.
- Use one uncertainty-readiness rule in measurements, controllers, stopping and UI:
  every stratum needs at least 32 attempts and two matches. Provisional means remain
  available; uncertainty is absent, not zero, before that threshold.
- Independence, stationary proposals and exact masses are documented statistical
  assumptions. No capability flags claim to certify them. Numerical inputs and
  normalized probability tables are still validated.
- Process initialization distinguishes fresh construction, exact restoration and
  training-to-inference handoff through protocol v7.

The [validation record](../validation/2026-10-21-stratification.md) covers queued
execution, graceful and abrupt recovery, physical transforms/materialization, and
controller handling of insufficient coverage. The [A/B report](../../benchmarks/stratification/README.md)
retains raw experiments and finite-sample uncertainty-calibration limitations.

Native observable bundles, quotas/adaptive allocation, learned centres, and
correlated-sampling error estimates are separate future extensions.
