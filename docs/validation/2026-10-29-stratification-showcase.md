# Tetrahedron and checkerboard stratification showcase — 2026-10-29

The [dashboard](http://itphlies:8098/) is also available through its
[Tailnet hostname](http://itphlies.tailb3264.ts.net:8098/).
It uses an isolated database on port 5418 and API on port 4018. Existing
ITPhlies deployments were left running. Both browser origins are allowed.

All four runs completed with eight local workers, split into one sampler and
three evaluators per concurrent run. Each has 524,288 pooled training samples,
fresh inference statistics from 4,194,304 samples, and a diagnostic PDF task.

| Run | Strata | PDF grid |
| --- | --- | --- |
| tetrahedron-ordinary | none | 4,097 points |
| tetrahedron-stratified | `(0,0)`, `(0,1)`, `(1,0)`, `(1,1)` | 4,097 points |
| checkerboard-sine-ordinary | none | 129 × 129 |
| checkerboard-sine-stratified | `(0)` negative, `(1)` positive | 129 × 129 |

Select `inference` to inspect contributions, primary/basic errors, allocation,
hit rates and variance shares. Select `pdf-diagnostics` and choose `Compare strata`
for conditional PDFs versus stratum targets; the checkerboard also has a 2D
membership map. Both runs retain ordinary counterparts for comparison.

The tetrahedron uses the checked-in smooth corner-dwelling path with exact vector
integral `[0,0,0]`. Its stratified result has 18 training and 16 inference history
frames. Its four native PDF integrals sum to one to floating-point precision.

The checkerboard integrates `sin(4*pi*x)*sin(4*pi*y)` on `[0,1]^2`. Centers `-1`
and `+1` split by sign. The exact total is zero, and the two contributions are
`-2/pi^2` and `+2/pi^2`.

| Checkerboard estimator | Estimate | Reported standard error |
| --- | ---: | ---: |
| Ordinary | 0.000305700 | 0.000204735 |
| Stratified | 0.000338974 | 0.000211113 |

The negative/positive contributions are `-0.202374894 ± 0.000149275` and
`+0.202713867 ± 0.000149284`. Their hit rates are 49.9503% and 50.0451%.
This example shows a limitation of factorized Havana densities: the checkerboard
sign depends on a correlation between coordinates, so either sign sampler still
visits both signs. These runs show similar errors, without the variance improvement
seen in the tetrahedron. Misses retain their zero contributions.

Browser checks exercised component selection, history metrics, density
normalizations, sampling mismatch, histograms, coverage, membership and JSON
exports. A reproducible canvas exception during rapid plot switching was fixed
by clearing pending drawing before the chart child disposes its canvas. The same
checkerboard browser sequence then passed without page-script errors. All 170
frontend tests passed, including a regression for cleanup order and forwarded
chart references; frontend lint and the production build passed.

Run cards, configurations, exports, screenshots, browser checks and test logs are
retained under `/common/dev/cedric/setup-logs/tetrahedron-demo-20261029/` on ITPhlies.
The [ordinary](../../resources/templates/runs/checkerboard-sine-ordinary.toml) and
[stratified](../../resources/templates/runs/checkerboard-sine-stratified.toml)
checkerboard cards are also checked in for reuse.
