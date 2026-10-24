# Stratification design decisions

Scaled Voronoi stratification is an integration sampling-pipeline feature. It
composes with campaigns/scans/tuning through their child integrations. The current
interface and complete estimator contract are in the
[execution specification](../execution-model.md#scaled-voronoi-stratification).

- Keep full integration weights on generated samples; query exact discrete
  probabilities once for frozen inference and derive conditional weights internally.
- Store the partition in sampling configuration, inherit it with trained proposals,
  and separate its left-bound prefix from the evaluator's integrand coordinates.
  Prefix lengths may vary by branch; centre-map keys exactly cover the domain.
- Gate values before accumulation; retain attempted counts and a default masked
  feedback view while preserving unmasked values and requested/actual prefixes.
  GammaLoop's primary integral is stratified while native histograms keep masked
  full-weight pooled statistics. Preserve grouping within each sampled draw.
- Expose the selected estimator independently of the partition. Training statistics
  remain pooled. Inference adds a pooled mean/error comparison derived from the
  existing conditional moments, without changing the primary control signal.
- Use an explicit fresh accumulator for inference. Immutable proposal identity,
  probabilities, and per-stratum moments survive splitting and checkpoint recovery.
- Use one uncertainty-readiness rule in measurements, controllers, stopping and UI:
  every stratum needs at least 32 attempts and two matches. Provisional means remain
  available; uncertainty is absent, not zero, before that threshold.
- Independence, stationary proposals and exact masses are documented statistical
  assumptions. No capability flags claim to certify them. Numerical inputs and
  normalized probability tables are still validated.
- Process initialization distinguishes fresh construction, exact restoration and
  training-to-inference handoff. Protocol v9 passes the full domain and rich
  feedback through one callback, without capability flags or full event records.

The [validation record](../validation/2026-10-21-stratification.md) covers queued
execution, graceful and abrupt recovery, integrand transforms/materialization, and
controller handling of insufficient coverage. The [A/B report](../../benchmarks/stratification/README.md)
retains raw experiments and finite-sample uncertainty-calibration limitations.
The [hybrid validation](../validation/2026-10-22-hybrid-stratification.md) covers
native/process histograms and the consolidated evaluation path.

Stratified histograms, mixed-estimator covariance/normalization, quotas/adaptive
allocation, learned centres, and correlated-sampling error estimates are separate
future extensions.
