# Stratification

Scaled Voronoi stratification is a sampling-pipeline option for integration tasks; campaigns, scans and
tuning use it through their child integration definitions. It is not a run kind.
Configure it with [Stratified inference](config.md#stratified-inference) and the
[tetrahedron](../resources/templates/runs/tetrahedron-native.toml) or
[GammaLoop](../resources/templates/runs/gammaloop-four-phases.toml) run card.
The [execution model](execution-model.md#ownership-and-flow) defines the shared
sampler/evaluator contract; [operations](operations.md#stratum-plots-and-pdf-inspection)
explains the dashboard controls.

- [Partition and domain](#partition-and-domain)
- [Training and inference](#training-statistics-and-the-selected-estimator)
- [Estimator terminology](#estimator-terminology)
- [Pooled comparison](#pooled-comparison-during-stratified-inference)
- [Observable statistics](#observable-statistics)
- [Statistical requirements](#statistical-requirements)
- [RSD and training ESS](#rsd-and-training-ess)
- [PDF diagnostic data](#stratum-plot-data-and-pdf-probes)

## Partition and domain

`sampler.config.stratification` contains `kind = "voronoi"`, a stratum-only
`domain`, a `centers` map, and optional `scales`. The prefix domain is independent
of the map, contains zero continuous dimensions, and contains no integrand axes.
Stratum dimensions form a left-bound prefix, which may have different lengths
on different branches. See [Domain](execution-model.md#samples-domains-and-queue-payloads)
for rectangular and branching representations.
GammaBoard appends the evaluator's integrand domain at each terminal prefix.
Rectangular prefixes remain compact when the integrand domain is rectangular.

Centre keys are canonical comma-separated nonnegative integers, such as `"0"` or
`"1,3"`. They must exactly match every complete stratum prefix declared by the
domain. Missing, extra, partial, and overlong keys fail validation. Centre values
are distinct finite vectors in unweighted evaluator-output units with exactly the
output width. Scales default to ones and must be positive. Classification minimizes
`sqrt(sum_j ((f_j-c_sj)/scales_j)^2)`; ties use numeric lexicographic prefix order,
independent of map insertion order. Scaling changes classification, not weights.

The sampler and materializer receive the augmented domain. After materialization,
the runtime separates the entire requested prefix, applies integrand transforms,
and evaluates the remaining coordinates once. Classification selects an actual
prefix. Accumulation and default feedback are masked on a mismatch, but unmasked
weighted training values remain available. Every miss counts as an attempted
sample and consumes budget.
Native GammaLoop and process observable evaluators use the same stratified
estimator for the real/imaginary integral and all histogram bins. Their centres
have two entries, `[real, imag]`; a real-part sign split uses
`centers = { "0" = [-1, 0], "1" = [1, 0] }` and a two-choice prefix domain.
Ordinary scalar/vector output uses its declared component layout.

## Training statistics and the selected estimator

Partitioning and estimation are independent choices. Training uses the pooled
estimator: **stratified statistics are not used during training**. All training
windows contribute, including earlier proposals. For component `f`, the pooled
sample is `Y_i = W_i f_i 1[requested_i = actual_i]`. Misses are zero observations.
For `N >= 2`, GammaBoard reports
`mean = sum(Y_i)/N` and `error² = sum((Y_i-mean)²)/(N*(N-1))`.
With one sample, the mean is provisional and the error is unavailable. Empty
accumulators have no estimate. The same sample-variance convention applies to
GammaBoard's pooled components, absolute means and discrete projections.

Training retains per-prefix attempts, matches, match fractions, and the observed
allocation `n_s/N` as diagnostics. Exact proposal masses are not queried for
adaptive pooled training and are not inferred from those frequencies. The result
panel distinguishes the partition, selected estimator, and training context.

Training uses `estimator = "pooled"` and full-weight projected feedback. Final
inference uses `estimator = "stratified"`, a supported sampler handoff and an
explicit fresh accumulator configuration. The partition is inherited from the
training source and cannot change during handoff. With joint sampler weight
`W = J / (p_k q(x|k))`, `k` identifies a complete prefix, `p_k` is its exact mass,
`q(x|k)` is the conditional proposal density for the remaining coordinates, and
`J` represents additional Jacobian factors. Define
`F(k,x) = f(x) * 1[classify(f(x)) = k]`. Inference accumulates
`Z_k = p_k W F(k,x)`: multiplying by `p_k` removes the discrete-probability factor
already included in `W`. The final sum does not apply it a second time.
Per-stratum attempted counts, means and central cross moments
merge using parallel Welford updates. The final mean is the sum of stratum means;
its covariance is the sum of `sample_covariance_k / n_k`. Component errors are
square roots of its diagonal. Norm training projection remains a separate scalar
observable: its mean is the integral of the norm, not the norm of the vector mean.

## Estimator terminology

Use **Pooled** for statistics over all weighted draws together, and **Stratified**
for the sum of conditional stratum estimates with errors combined in quadrature.
A **Stratum contribution** is one conditional estimate. **Combined** describes
the view containing all strata; **Independent sum** (`independent_sum` in data)
describes a campaign's weighted sum of independent run estimates.

**Error** means one standard error. Explicit fields use `pooled_mean`,
`pooled_error`, `stratified_mean`, and `stratified_error`; `error_reduction` is
`pooled_error / stratified_error`. Generic `{mean,error}` values use the Rust
`MeanEstimate` type. The optional comparison field is `pooled`, including in
measurements, plots, and histogram bins; tuning uses `objective_pooled`.

Pooled describes an estimator, not whether samples have a stratum assignment.
The pooled comparison in a stratified run retains masking and counts misses as
zeros. An **unstratified run** is a separate run without stratification. Masked
and unmasked describe sample values and feedback, not alternative names for these
estimators. No unmasked observable estimator is implied by the Pooled label.

## Pooled comparison during stratified inference

The primary estimate remains the stratified mean and its conditional error.
The `pooled_mean` and `pooled_error` comparison describes the pooled estimator of
**the same masked, fully weighted draws**. This is a different mean, and does not
predict a separately trained unstratified sampler's performance.

For component moments, the pooled comparison is reconstructed from the same
per-stratum statistics. For stratum count `n_s`, conditional mean `m_s`,
central moment sum `M2_s`, and frozen mass `p_s`, its pooled group has mean `m_s/p_s`
and central moment sum `M2_s/p_s²`. These groups are merged with the stable parallel
mean/covariance formula, including between-group mean differences. The resulting
pooled error uses `N*(N-1)`. A pooled comparison can be available while the primary
stratified error is still unavailable; it is never used as a fallback for stopping.

Measurements carry `estimator` (`pooled`, `stratified`, or `independent_sum`), primary
`value`/`uncertainty`, and optional `pooled: {mean,error}`. Controllers and tuning
continue to use only the selected primary estimate. Campaigns combine selected
child estimates and their uncertainties; any pooled comparison is combined
separately with the same coefficients. Relative errors and target comparisons
refer to the corresponding primary mean and error.

The same identity and comparison travel through history, JSON/CSV export, plots
and full-precision disclosures. Plot points retain the explicit primary error;
reconstructing it by subtracting rounded band endpoints can destroy small errors.
Missing errors are null/blank or labelled unavailable, never converted to zero.
See [reading results](operations.md#read-results-and-plots) for display notation.

Retained diagnostic points include requested and actual prefix tuples, integrand
coordinates after materialization/transforms, and total integration weights.
Training extrema describe pooled `W*f`; stratified extrema describe conditional
`p_s*W*f`, with their basis labelled. Observable producers' raw real/imag values
allow the host to retain complete prefix metadata without changing native moments.

For discrete-coordinate projection diagnostics, the host marginalizes over the
complete prefix set and offsets each integrand query by that prefix's length.
An unspecified joint query leaves the marginal unspecified. Custom process
materializers/transforms can remap discrete coordinates, so their integrand-space
probability diagnostic remains unspecified without a known coordinate mapping.

## Observable statistics

Let `g_ib` be the sum of all correlated events/cuts/counterterms from original
draw i in bin b (including underflow or overflow). For requested stratum k,
accumulate `Z_ikb = p_k W_i g_ib 1[requested_i = actual_i]`. Every attempt counts
in n_k, including misses and draws with no entries in that bin. Then

```
I_b = sum_k mean_kb
Var(I_b) = sum_k sum_{i in k}(Z_ikb - mean_kb)^2 / (n_k (n_k - 1))
```

Merge raw bundles from batches/workers within each stratum before combining these
independent estimates. Never pool raw moments across strata to estimate I_b.
GammaBoard's [reducer](../src/evaluation/reduction.rs) applies the mask and weights;
its [result projection](../src/services/results.rs) calls GammaLoop's
`sum_independent_estimates` helper to sum the stratum means and error variances.
The checkpoint retains each stratum's raw bundle and the masked full-weight pooled
reference; all histogram sample counts must match their respective attempted counts.
Training uses only the pooled estimator and resets for frozen inference.
Empty strata make the final mean unavailable. Errors use the same coverage guard
as the primary integral (32 attempts and two matches per stratum).

Presentation payloads retain raw reference statistics and add per-bin `estimate`
(mean/error), `pooled` (pooled mean/error), and per-stratum sample counts. Dashboard,
campaign combinations and exports use the selected estimator. JSON preserves
unavailable estimates; HwU requires available means/errors. No fictitious pooled
sample count or raw moments are manufactured for a sum of stratum estimates.
Bin errors do not specify covariance between different bins: uncertainty on a sum
of bins, ratio or normalization needs the corresponding joint statistics.
CDF views show cumulative means without error bars until that covariance is available.
Display-only [bin merging](operations.md#observable-comparisons) instead uses an
explicit uncorrelated-bin approximation; it does not create joint statistics.

Inspect the estimates with the [observable controls](operations.md#observable-comparisons).

For a single stratum, the pooled comparison is its contribution to the pooled
estimator over all N draws: `S_kb/(p_k N)`, with error squared
`(Q_kb - S_kb^2/N)/(p_k^2 N(N-1))`. Other strata's draws are zeros in this comparison.
It uses the same samples, rather than a separately trained unstratified run.
Conditional errors use that stratum's own coverage readiness; combined errors
require all strata to be ready. Adaptive training retains the ordinary pooled view.

## Statistical requirements

For each complete prefix, the runtime queries `discrete_mass` with every prefix
coordinate fixed and all remaining coordinates marginalized. Arbitrary other
marginals may be unavailable. Training and explicitly pooled estimation do not
require these queries. Stratified inference fails if any required mass is
unavailable; it never estimates masses from counts or silently switches estimators.
Before inference generates work, the runtime validates exact finite positive
masses summing to one within `1e-12`. It does not renormalize guessed probabilities
or impose probability floors afterward. The immutable plan includes component
layout, partition, probabilities and a proposal identity; batches/checkpoints
carry it and incompatible accumulator merges fail. Recovery restores the same
plan without querying a newer proposal. A new inference stage gets a new identity
and fresh statistics. The runtime relies on independent frozen sampling as a
documented assumption, not a capability handshake.

An unsampled stratum makes the combined estimate unavailable. Covariance and
standard errors remain unavailable until every stratum has at least 32 attempts
and two matches. This single publication rule applies to measurements, campaign
allocation/convergence, task stopping, live panels and restored history. Means can
be displayed before their uncertainty is available; unavailable uncertainty is
never encoded or displayed as zero. Fixed budgets remain hard limits.
Zero matches are reported explicitly: even many zero observations do not establish
that a stratum's integral is zero. Error bars assume finite second moments and may
be unreliable before rare large contributions have been sampled. Quotas, adaptive
inference and correlated/QMC error estimation are outside this implementation.

Diagnostics distinguish exact proposal masses from observed allocation and retain
requested/actual prefixes, attempts and matches. RSD uses the primary estimate;
training ESS uses the per-stratum definition below. Generic pooled variance and
ESS are not substitutes for stratified uncertainty. Combined means/errors feed
the normal measurement and controller paths.

The coverage threshold does not guarantee that rare importance-weight tails have
been observed. At a finite budget, sampled variances can substantially underestimate
the proposal's true variance even when the estimator contract is satisfied. See the
[prefix-model validation](validation/2026-10-22-stratum-prefixes.md) for an observed
example and an independent frozen-proposal variance calculation.

## RSD and training ESS

RSD is `sqrt(N) × SE(estimate) / |estimate|`, using each signed component of the
primary integral. Stratified runs use their conditional-estimate error, not the
pooled reference error. RSD is dimensionless and stays roughly constant during
inference with a fixed proposal. Cancellation to a zero estimate produces an
infinite RSD when the error is nonzero; `0/0` is undefined. Insufficient coverage
leaves it unavailable. Only finite values enter the RSD history plot.

Normalized training ESS is `(Σ|w|)² / (N Σw²)`, between zero and one, where `w`
is the weighted **training projection**. For partitioned sampling, calculate it
within each requested stratum, counting mismatches as zero contributions. The
aggregate is `Σ(N_s × ESS_s) / ΣN_s`; stratum-specific values appear under
**Stratum output**. This measures training-weight concentration, not the variance
of the primary stratified integral, and is not inferred from RSD.

## Stratum plot data and PDF probes

Stratum plot histories project the existing accumulator snapshots. Each frame
contains cumulative attempted/matched counts, optional exact masses, per-component
conditional estimates when selected, and the unchanged primary total. No new
per-draw collection is required. Variance shares are `error_k² / sum(error_j²)`;
all shares remain unavailable if any error is unavailable or the total reported
variance is zero. Reporting-interval allocation and hit rates use differences of
adjacent retained cumulative snapshots. Hiding a curve never recomputes totals.

A PDF diagnostic is a read-only stage handoff. It restores the saved proposal,
inherits its output layout/training projection into a fresh accumulator, and
uses the existing feedback path. For a stratified source it evaluates a common
sampler-coordinate raster once, carrying the first complete prefix for routing.
The evaluator strips that prefix and returns the unmasked weighted projection
and actual classification. The raster has unit sampling weight; transformations
supply any Jacobian factors. The diagnostic stores these unmasked projections and
actual labels, rather than consuming the generic masked feedback view.

For every prefix `k`, it queries `pdf_batch` at `(k ++ integrand_discrete, u)` and
`discrete_mass` for the complete prefix. Stored per-stratum PDFs are joint
`p(k,d,u)` values; the overview is their sum. Conditional PDFs are
`p(k,d,u)/p_k`, conditional only on `k`, not additionally on a fixed integrand
discrete path. Unknown values remain absent. The target is the absolute source
projection times `1[actual=k]`; the density remains unmasked. PDF values, actual
assignments and raster progress survive checkpoint/replay in canonical grid order.

All plotted values use the same sampler-coordinate slice and transformed
integrand measure. Custom process materializers cannot currently be inverted or
probed by this interface and fail explicitly. Probes use the ordinary PDF/mass
queries without a separate stratification capability declaration. Native uniform
sampling returns its joint PDF on
rectangular and ragged discrete domains as well as purely continuous domains.

Display-only Slice normalization divides each field by its mean over the
evaluated grid. Slice-integral/leakage diagnostics retain trapezoid quadrature in
line parameter `t` or plane parameters `(u,v)`. Native Havana line diagnostics also
retain the union of intersected grid edges and the joint PDF on each resulting
interval. This provides exact piecewise-constant rendering and bin integration
without additional integrand evaluations or a new sampler API. Generic samplers
retain sampled-point rendering and quadrature. Checkpoints retain the optional
step representation and source projection label with the raster results.

A finite positive integral is required for normalization. A source-reference
comparison uses an available estimate of the absolute training-projection
integral; it never substitutes the magnitude of a signed integral. Reference
normalization does not guarantee unit area on a slice. Leakage and actual-cell
mass readouts use classified raster quadrature and are explicitly slice estimates.

For ratios, positive target with zero PDF is uncovered, positive PDF with zero
target is leakage/zero target, both zero is undefined, and missing inputs remain
unavailable. Numeric curves omit infinite/undefined ratios while the Coverage
colors and counts preserve those cases. Diagnostic histograms are distributions of
finite raster values, weighted by the selected joint PDF by default (or uniformly
when weighting is disabled). These are not independent Monte Carlo observations
and carry no Poisson or Monte Carlo error bars.

JSON/CSV diagnostic exports retain normalization metadata, raw queried densities,
actual prefixes and diagnostic statuses. Native PDF intervals remain in persisted
raster output and are used for line-plot rendering.
View settings do not change the stored statistics.

Stratum histories use snapshot timestamps for elapsed time and the latest
preceding sampler performance report for the same task for reported sampler time.
Reports from other tasks or later timestamps cannot be substituted. Missing time
metadata remains absent, and merging a current frame cannot erase an existing
historical timestamp. Counts remain the canonical history key, independently of
the selected horizontal axis.

## Design and validation

Stratification composes with existing sampling, domain and feedback interfaces; it
does not require a special sampler class. Full integration weights remain on
samples, while the runtime derives conditional weights from exact prefix masses.
A partition and proposal remain fixed within an inference stage. Centres can be
tuned across independent runs, with fresh statistics for every candidate.

For process samplers, the [standalone checker](../process_api/README.md#check-a-sampler)
can exercise exact prefix masses and known per-stratum/combined toy integrals with
`--require-stratified --statistics`.

The [validation archive](validation/README.md#stratification-and-observables) links
the implementation and recovery checks. The [A/B benchmark](../benchmarks/stratification/README.md)
retains measured comparisons and finite-sample error-calibration limits. Historical
reports describe the code at their recorded revision, including superseded designs.
