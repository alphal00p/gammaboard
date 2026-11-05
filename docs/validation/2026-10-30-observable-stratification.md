# Per-sample evaluation and observable stratification

GammaBoard now pins GammaLoop `227e3534a5fb39f64710b6754cc63f34ed4c95a7`
(the sample-observable separation merged in PR #242).

Evaluators emit one unweighted output per input. Runtime reduction owns outer
weights, masks, feedback and all multi-sample statistics. The Python callback is
`eval(...)`, returning component arrays or `ObservableBatch`. The old aggregate
callback and MadGraph native-integration trigger wrapper were removed; the
MadSpace per-sample evaluator remains. Native GammaLoop events are consumed one
sample at a time using a separate reusable observable runtime.

Each observable bin, including flows, retains conditional raw statistics per
stratum. Attempts include misses and empty fills; correlated event contributions
within a draw are summed before squaring. Inference coefficients are `p_k * W`
on a match and zero otherwise. Final means and error variances are summed across
strata after merging batches within each stratum. Training remains pooled. Pooled
reference statistics are retained and labelled separately. Display, campaign
combination and exports use the chosen estimator; unavailable errors stay absent.

Validation:

- `cargo test --locked`: 374 library tests, 8 binary tests, 4 default full-stack
  tests and 1 doctest passed. Optional integration suites remain ignored here.
- Five dedicated full-stack stratification tests passed against disposable
  PostgreSQL: Havana checkpoint recovery, process handoff with exact masses,
  materializer/transform Jacobians, PDF diagnostics, and abrupt sampler/evaluator
  termination with outstanding work. Recovery compares every conditional
  histogram bin's raw sums, squared sums and sample counts to a replayed stream.
- Two Rust/Python process round-trip tests passed with protocol v10. They check
  pooled and conditional observable moments and feedback weights.
- Native GammaLoop acceptance checks compare conditional histogram estimates with
  direct per-stratum references, including misses, unequal masses, split batches
  and JSON restoration. Correlated fill and underflow/overflow fixtures also
  check campaign combinations and actual HwU output values/errors.
- Python: 34 passed; one optional test requiring a MadGraph installation skipped.
- Dashboard: 176 passed; lint and production build passed. A regression checks
  stratified bin values override pooled sums and incomplete coverage stays unknown.
  CDF views no longer assume independent bins when estimating cumulative errors.
- Strict Clippy, formatting and a build without default features passed.

Compatibility: database model 8, checkpoint model 6, process protocol v10. Queue
input envelope 4 is unchanged. Use a fresh database and matching workers/SDK.
During validation, older installed v9 adapters were correctly rejected; rerunning
with the matching SDK passed. The dashboard build used an isolated output folder
because the existing build directory is protected. Existing demo services were
not redeployed.

Limits: cross-bin covariance is not stored, so bin errors alone do not determine
uncertainty for sums of bins or normalized ratios. The independent frozen-proposal
assumptions remain the sampler's responsibility. Native event memory is bounded
by one sample, while external-process observable batches carry per-sample bundles;
large histogram schemas may require smaller process batches. No throughput claim
is made from these correctness tests.

## Histogram population and estimator selectors

Frozen-inference observable histograms now have a searchable Combined/per-prefix
selector and four display modes: Estimate, Compare estimates, Compare errors,
and Error reduction. Comparisons use at most two curves on one plot. They retain
separate central values and use the same draws for the basic (unstratified)
reference. Individual basic contributions use the full draw count including zeros
from other strata. Coordinates are shared across the lightweight per-stratum
estimate payloads. The runtime/checkpoint model is unchanged by these controls.

Selections persist alongside zoom/scale preferences and are included in figure
JSON/CSV. Incomplete errors and zero ratio denominators remain gaps; error views
hide value-only controls and uploaded value references. Independent uploaded
comparisons remain available in the combined estimate view. Browser checks with
24 ragged-prefix choices covered all display modes, discrete-series replacement,
and widths 1280, 768 and 390 pixels. A grid-sizing fix prevents canvas clipping
when narrowing the window. Focused backend tests cover per-stratum normalization,
result construction and panel projection; frontend tests cover selections,
missing/zero errors, discrete bins, references and exports.

Selector validation passed: 183 dashboard tests, 15 focused backend tests, strict
Clippy, frontend lint and production build. The temporary preview server and its
source files were removed after browser verification; existing deployments were
not changed.
