# Statistics and presentation consistency — 2026-10-22

This pass separates the partition from the selected estimator throughout results,
measurements, plots, controllers and exports. Training remains pooled, including
earlier proposals; the dashboard explicitly says that stratified statistics are
not used during training. Inference retains the stratified estimate as primary
and derives a pooled `basic_mean`/`basic_err` comparison from the same conditional
moments. Each uncertainty is paired with its own mean. The comparison never
replaces a missing primary error or changes stopping decisions.

Pooled standard errors use the sample-variance correction `N-1`. Below two
observations, the mean remains visible and the error is unavailable. Stratified
coverage requirements remain distinct from warnings about rare large weights.
Neither the coverage threshold nor a smaller pooled comparison certifies that the
tails have been adequately sampled.

The interface displays same-scale absolute uncertainties, for example
`1.5(0.5)`. The full-precision disclosure includes estimator identity and both
estimates when available. Histogram statistics remain explicitly pooled, and
training projections are labelled separately from the integral components.
Retained diagnostic points include requested and actual prefixes, coordinates and
total weights. Training diagnostics distinguish observed allocations from exact
proposal masses; unavailable queries are not presented as zero probabilities.

## Validation

| Check | Result |
| --- | --- |
| Rust library suite | 372 passed; one optional external-state test ignored |
| Final result-panel checks | 11 passed after the last presentation changes |
| CLI contract checks | 4 passed |
| PostgreSQL-backed core workflows | 37 passed |
| PostgreSQL store integration | 42 passed |
| Cluster store integration | 6 passed |
| Rust/Python process roundtrips | 2 passed |
| Python SDK, MadGraph and recovery fixtures | 42 passed, one optional test skipped; four subtests passed |
| Frontend | 155 passed across 18 files |
| Static checks | Rustfmt, strict Clippy with all features and without default features, frontend lint, whitespace checks passed |

The core workflow suite includes frozen-proposal handoff, sparse/ragged prefixes,
materialization and transforms, checkpoint replay, abrupt recovery and campaigns.
Seven separately installed external-adapter workflows were excluded. New targeted
checks compare reconstructed pooled moments with direct accumulation, preserve
estimator identity during campaign combination, and ensure that a missing
stratified error cannot fall back to the available pooled error. Frontend checks
cover explicit decimal uncertainties, unknown errors, full-precision disclosures
and CSV exports.

## Live dashboard

The [dashboard](http://itphlies.tailb3264.ts.net:8097/) was rebuilt against a fresh
database model 6 and checkpoint model 5. The earlier model-5 database and binary
are retained separately. Process protocol v9 and queue format 4 are unchanged.
The four sine/tetrahedron examples were rerun with ten workers and completed at
their exact budgets:

| Pair | Training attempts per arm | Inference attempts per arm |
| --- | ---: | ---: |
| Tetrahedron | 131,072 | 131,072 |
| Sine | 33,554,432 | 268,435,456 |

For the stratified tetrahedron, the expanded display and downloaded CSV agree:

| Component | Primary mean | Stratified error | Basic mean | Basic error |
| --- | ---: | ---: | ---: | ---: |
| x | 0.000205864 | 0.000596271 | -0.002272833 | 0.002679498 |
| y | -0.002353145 | 0.000615352 | -0.002178443 | 0.002672962 |
| z | -0.000288254 | 0.000635673 | 0.001207580 | 0.002536757 |

The different means are expected: conditional means use each stratum's count;
the pooled comparison uses the total count. This comparison uses the stratified
proposal's draws and is not the result of the separately trained ordinary arm.
The previously measured rare-weight limitation in the tetrahedron y component
still applies; see the [independent variance check](2026-10-22-stratum-prefixes.md#observed-uncertainty-limitation).

Headless Chromium checks exercised training and inference disclosures, all four
tetrahedron prefix labels, retained requested/actual labels, JSON/CSV downloads,
and both sine target comparisons. A temporary one-sample constant run verified
that `1.5` stays visible with an unavailable error. No browser page errors were
recorded. The temporary run was removed after retaining its evidence.

Build logs, run cards, exported estimates and browser captures are retained on the
execution host under
`/common/dev/cedric/setup-logs/statistics-consistency-20261022/`.
