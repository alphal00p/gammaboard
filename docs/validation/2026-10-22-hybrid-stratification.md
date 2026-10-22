# Hybrid stratification validation — 2026-10-22

GammaLoop native and process observable output can now combine a stratified
real/imaginary integral estimate with ordinary pooled histograms. Classification
uses unweighted primary values. Both streams use the same mask and attempted
samples; only the integral receives the frozen `p_k` correction. Histograms retain
full weights and group correlated events within each original draw before taking
second moments. Their totals need not equal the stratified integral estimate.

Native evaluation uses existing single-sample GammaLoop entry points and commits
observables after classification. It does not reevaluate points or transport event
records. The process protocol carries the masking context and raw real/imaginary
values; observable bundles remain compact batch summaries. An adapter launching
an independent integration from one trigger cannot satisfy the per-draw contract.

## Consolidation

- One default evaluator path handles scalar/vector values, weighting, masking,
  collection and training feedback. The runner no longer branches between ordinary
  and stratified evaluation calls.
- Shared plan initialization handles fresh inference and frozen recovery.
- Native and process adapters share hybrid finalization and count validation.
- Histogram/diagnostic access is independent of the primary estimator. Shared
  metric extraction drives real/imaginary history, and pooled extrema rendering
  no longer duplicates scalar/vector versus GammaLoop branches.
- The execution specification, wire contract, compatibility table and Python
  producer example describe the same interface. A hybrid run card is included.

## Evidence

- **367 library tests pass**, with one pre-existing external-state test ignored.
  This includes the existing repeated-sampling calibration and unequal-probability
  covariance tests, plus a real generated GammaLoop process exercising conditional
  integral moments, full-weight native histograms, misses, grouped event squares,
  fresh training, split/merge equivalence and JSON restoration. Malformed histogram
  counts are rejected without changing the destination accumulator.
- **41 backend workflow tests pass**, including controllers, failure policy,
  seeded recovery, native Havana pause/resume, process sampler stage handoff and
  nontrivial materializer/transform Jacobians. The stratified SIGKILL case now
  recovers the hybrid state and compares both its conditional moments and every
  pooled histogram bin against an independent replay of the frozen sample stream.
- **42 PostgreSQL checks pass across the full run and targeted fixture rerun**.
  The compatibility fixture was changed from a hardcoded model number to the
  current version constant; the other 41 checks passed in the full run.
- **Six cluster-store tests pass**, covering worker build fencing, launch leases,
  late registration and deployment replacement with the new database model.
- **Two real Rust/Python process roundtrips pass**, including unequal stratum
  probabilities, retained misses, full-weight feedback and pooled bin moments.
- **38 Python SDK/MadGraph tests pass**, with one optional external-state test
  skipped. Mask tests exercise scaled distances, ties, invalid inputs and the
  exact feedback/value binary ordering.
- Strict Clippy for all targets passes both with all features and without default
  features. Formatting, changed-file Python/TOML syntax, documentation links and
  whitespace checks pass.

The broad regressions caught two issues during consolidation: the reference
overlay's imaginary normalized moment needed its existing reporting entry point,
and scalar `full_vector` collections needed to retain requested training feedback.
Both were corrected, and the library and complete backend suites were rerun.

## Browser check and compatibility

An isolated localhost deployment ran the documented polynomial observable example
with 16,384 training and 65,536 inference attempts. The browser displayed real and
imaginary central estimates, per-stratum coverage, pooled histogram labels and
bins, and pooled weight diagnostics without page errors. The test instance was
stopped and its database removed afterward. Existing live demonstrations were
left on their pinned build and database.

The hybrid persisted shape and observable callback contract require **database
model 4, saved/checkpoint model 3, queue GBQ3 and process protocol v8**. Upgrade
server, workers and SDK together using a fresh database. No compatibility adapter
was added. Full histogram stratification and covariance for normalizing histograms
by the stratified central estimate remain outside this hybrid's contract.
