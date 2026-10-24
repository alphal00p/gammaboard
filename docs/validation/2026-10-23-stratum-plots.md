# Stratum plots and PDF showcase — 2026-10-23

The dashboard now projects per-stratum histories and breakdowns from existing
accumulator snapshots. Conditional inference exposes component contributions with
reported uncertainty and the unchanged total. Training exposes only counts,
allocation and hit rates. Variance shares use squared errors and require all
stratum errors to be available. Prefix labels and colours remain consistent across
the new views, including sparse/ragged prefixes.

PDF line and plane tasks inherit a saved sampler's partition and inspect every
complete prefix on a common raster. The evaluator runs once per raster point;
the diagnostic queries the source PDF for each prefix, retains the unmasked
training projection and actual membership, and preserves density outside the
intended stratum. Display modes distinguish slice-normalized shapes, conditional
densities and joint densities. PDF tasks inherit the source component layout and
projection into fresh statistics; they do not resume the source accumulator.

## Validation

- 375 Rust library tests passed; one optional external-state test was ignored.
  An earlier run hit a pre-existing short-timeout I/O scheduling test under load;
  it passed in isolation and in the final full run with four test threads.
- All 38 PostgreSQL-backed core workflow tests passed, including the new
  multi-evaluator line/plane diagnostic workflow. Seven external-adapter workflows
  requiring separate installations were excluded.
- The new workflow verifies the vector norm projection, actual classification,
  complete prefix table, joint densities and exact grid budgets through the queue.
- A native test uses `(0)`, `(1,0)`, `(1,3)` with additional integrand discrete
  coordinates, unequal masses and partial checkpoint replay. It verifies canonical
  output ordering, unmasked misses and the joint/conditional probability distinction.
- Plot tests compare every vector component's total error with the primary
  measurement, including the extra norm-projection covariance dimension.
- 161 frontend tests passed. New cases cover component/view interactions,
  squared-error shares, missing errors/masses, slice quadrature, leakage,
  reporting-interval counts and exports.
- Strict Clippy passed with all features and with default features disabled;
  frontend lint, production build, Rustfmt and whitespace checks passed.

Browser validation exercised component selection, total visibility,
allocation/hit-rate history, reporting intervals, training labels, all three PDF
normalization modes, logarithmic/linear scales, individual/all-stratum selection,
two-dimensional membership and JSON/CSV/SVG downloads. The exported tetrahedron
total errors were checked against the quadrature sum of the stratum errors.
No page-script errors were recorded.

## Showcase

Open the [dashboard](http://itphlies.tailb3264.ts.net:8097/):

| Run | Task | View |
| --- | --- | --- |
| `tetrahedron-stratified` | `inference` | x/y/z contributions, total, allocation, hit rates and variance shares |
| `tetrahedron-stratified` | `train` | Pooled training allocation and hit-rate histories |
| `tetrahedron-stratified` | `stratum-pdf` | Four prefix-labelled conditional PDFs versus masked norm targets |
| `sine-stratified` | `stratum-pdf` | Negative/positive strata, corresponding targets and PDF leakage |
| `strata-pdf-showcase` | `stratum-pdf` | Signed two-bump example: membership map and target/PDF heatmap pairs |

The sine and tetrahedron tasks reuse the existing frozen inference snapshots,
with 4,097 new diagnostic evaluations each. The new two-dimensional run completed
131,072 training attempts, 262,144 inference attempts and a 65×65 PDF raster.
Ten workers supported the three runs with up to seven simultaneous evaluators.
Existing A/B results remain available. The database/checkpoint/queue/process
versions remain 6/5/4/v9; no statistical accumulator or wire layout changed.

The short tetrahedron inference has only two stored history snapshots. For a
detailed convergence demonstration, use the sine inference (190 snapshots); the
new two-dimensional run also has 21 training and eight inference snapshots.

## Limits

These plots are slices, not marginals over omitted coordinates. Shape
normalization requires a complete finite grid and a positive trapezoidal integral;
zero or missing data remain explicit. Raster membership is only resolved at grid
points. Log scales omit zero densities. Known-zero and unspecified prefix masses
cannot produce conditional curves, but joint-density data remain available.

Custom process materializers are rejected because the current PDF interface does
not supply their inverse coordinate mapping. Standard batch transforms still
apply, including their Jacobians in the importance target. No scatter renderer,
streaming output-point collection, requested/actual confusion matrix or
stratified observable-bin accumulation was added in this pass.

Logs, run cards, exported data, browser checks and screenshots are retained under
`/common/dev/cedric/setup-logs/strata-plots-20261023/` on the execution host.
