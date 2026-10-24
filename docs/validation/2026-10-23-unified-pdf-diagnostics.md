# Unified PDF diagnostics — 2026-10-23

Ordinary and stratified PDF tasks now share one backend projector and one
comparison panel. The separate legacy normalization, mismatch and histogram
projection path was removed. The client uses the same diagnostic calculations,
coverage handling, normalization and export logic for both cases.

The default is one overview. Scope selection offers individual prefixes and
comparisons paged four at a time; lists with more than twenty options support
search. View selection shows densities, mismatch, distributions, coverage or
stratum membership. Normalization/leakage tables and colour controls are
expandable. This bounds the number of rendered figures without discarding strata
or changing totals and variance-share denominators.

## Correctness and controls

- Ratios use comparable slice-shape or source-reference normalization. Positive
  target/zero PDF, zero target/positive PDF, both zero and missing inputs have
  explicit coverage states. Infinite/undefined ratios remain counted when omitted
  from numeric plots. Log ratio is the default mismatch view.
- Diagnostic histograms use deterministic grid-point frequencies, common bin
  edges, and no Monte Carlo/Poisson error bars. The existing histogram CDF, scale,
  ratio and export controls are reused.
- Havana line diagnostics retain the union of native bin edges and the density
  on each interval. Curves and PDF normalization use that exact representation;
  no extra integrand evaluations or sampler-protocol methods are required. Other
  samplers retain grid queries and quadrature.
- Details expose the projection label, raw target/PDF slice integrals, divisors,
  exact requested prefix mass, target share, actual-cell mixture mass and
  outside-cell proposal mass. Cell masses/leakage remain grid estimates on the
  selected slice. Source-reference mode uses an available absolute-projection
  integral estimate and never substitutes the magnitude of a signed integral.
- Both logarithmic colour limits are shared across comparable heatmaps. Manual
  limits, automatic limits and linked x/y navigation persist across view changes.
  Raw PDF and raw target quantities have separately labelled colour scales.
- Stratum histories support sample count, elapsed time and reported sampler time,
  plus follow-latest and persistent zoom. Sampler time comes from the last
  preceding performance report for the same task; later/other-task reports are
  excluded and missing times are not invented.
- JSON/CSV exports retain raw grids, native PDF intervals, display settings,
  normalization diagnostics, coverage status and timing fields. Integral totals
  remain independent of displayed strata.

## Validation

- 366 Rust library tests passed; one optional external-state case was ignored.
  The reduction from the preceding suite reflects removal of tests for the deleted
  legacy PDF calculation path. A new test checks a narrow native bin that a coarse
  raster would miss and verifies the exact PDF integral.
- All 38 PostgreSQL-backed core workflows passed, including multi-evaluator
  ordinary sampling and stratified PDF line/plane tasks. Seven separately
  installed adapter workflows were excluded.
- A PostgreSQL test serializes the worker report context and verifies that
  snapshot timing uses only preceding reports from the same task. Reports before
  any matching telemetry retain unavailable sampler time.
- 168 frontend tests passed, with 60 affected tests run after final display
  changes, including an additional log-ratio overflow regression. Tests cover ordinary/stratified normalization, zero cases, exact-bin
  rendering data, shared log limits, manual bounds, persisted linked zoom,
  distributions without error bars, paged comparisons and searchable long lists.
- Strict Clippy checks passed with all features and without default features;
  Rustfmt, frontend lint, production build and whitespace checks passed.

Browser checks exercise ordinary and stratified density comparisons, both
mismatch metrics, coverage, distributions, manual/shared colour limits,
membership, time-axis selection, reload persistence and JSON/CSV/SVG export.
Native-bin integrals exported from the sine and tetrahedron examples are checked
against one to floating-point precision.

## Live examples

Refresh the [dashboard](http://itphlies.tailb3264.ts.net:8097/) after deployment.
The new task is named **pdf-diagnostics** in each of these runs:

| Runs | New diagnostic evaluations |
| --- | ---: |
| sine-ordinary, sine-stratified | 4,097 each |
| tetrahedron-ordinary, tetrahedron-stratified | 4,097 each |
| strata-pdf-showcase | 65×65 |

All five tasks reuse their existing frozen source samplers and leave the A/B
integration results intact. The source data and process protocol remain compatible
with database/checkpoint/queue/protocol versions 6/5/4/v9. Older diagnostic
snapshots can lack optional projection labels and native step data; the shared
view explicitly uses the available queried grid in that case.

Evidence is retained on the execution host under
`/common/dev/cedric/setup-logs/pdf-unified-20261023/`.

## Release review — 2026-10-24

The final accumulated-diff review removed unused normalization and image-zoom
helpers, unused PDF projector parameters and unnecessary masked-feedback
allocations. Comparison paging now uses one shared page size and clamping rule;
quadrature tests exercise the actual comparison path. The new diagnostic modules
were formatted for readability, and the changelog, protocol guide and clean-wheel
CI check were aligned with protocol v9 and persisted formats 6/5/4.

The review also caught a Python Havana construction regression: its internal grid
builder still received cardinalities and dimensions after its signature had been
changed to a domain. The helper now retains its internal shape-based contract,
while public callbacks take `Domain`. Havana and MadNIS adapter regressions now
pass `Feedback` objects and exercise the current API.

Final validation used a fresh PostgreSQL 17 instance with the consolidated baseline
schema, separate from the showcase deployment. It passed 366 Rust library tests,
43 database tests, six cluster-store tests, 38 core workflows with recovery seeds
17 and 41, and two Rust/Python process round trips. Strict Clippy passed with all
features and with default features disabled. All 169 frontend tests, lint, the
production build and the production dependency audit passed. Python checks
included a clean installed SDK
wheel on Python 3.13, MadGraph, operational and benchmark tests, the four MadNIS
training regressions and the Havana snapshot regression with pinned Symbolica
1.5.1. All 52 tracked TOML definitions parsed successfully.

The optional tests requiring external physics state and seven external
adapter workflows were not part of this final run. Browser evidence above remains
the validation of the deployed showcase; this review did not redeploy it. Logs and
the isolated build outputs are in
`/common/dev/cedric/setup-logs/stratification-release-20261024/`.
