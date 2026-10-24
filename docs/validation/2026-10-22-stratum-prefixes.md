# Stratum-prefix and feedback validation — 2026-10-22

The sampler protocol now exposes complete domains and one rich feedback object.
`Integrand` and `Stratum` roles are left-bound on each path; rectangular nodes use
`strata_dims`. Prefix lengths may vary, and explicit branch labels may have gaps.
Voronoi centre maps must exactly cover the independently declared prefix domain.
Generic samplers consume GammaBoard's masked view. Unmasked weighted projections
and requested/actual prefixes remain available to specialized training algorithms.

The probability operation is `discrete_mass`, with unspecified entries allowed.
Exact masses are required only for the complete prefixes used by stratified
inference. Joint PDFs are normalized; `global_pdf_norm` has been removed.
The process protocol is v9, database model 5, sampler/checkpoint model 4, and queue
model 4. These are breaking changes; the validation uses fresh databases.

## Interface, statistics, and recovery checks

- 369 library tests passed; one pre-existing optional external-state test was skipped.
  This includes the 512-repetition uncertainty calibration, native GammaLoop
  histogram accounting, and scalar/vector inference.
- 41 sampler/materializer/transform tests passed again after the final feedback
  recovery consolidation. The existing split-draw checkpoint test now preserves
  raw values and ragged requested/actual assignments through partial feedback.
- Four CLI contract checks and all 37 database-backed core workflow tests passed across the main run and targeted
  follow-ups. The local-launch replacement test timed out under parallel load and
  passed in isolation. Its behavior should continue to be watched under load.
  Seven external-adapter workflows requiring separate installations were excluded.
- The queued Havana pause/resume case now uses `(0)`, `(1,0)`, `(1,1)`, `(1,3)`.
  Native tests also verify sparse-label probability queries, complete centre
  coverage, generic masked training, unmasked misses, and frozen replay.
- The remaining stratification workflows cover process stage handoff, unavailable
  unrelated marginals, materializer/transform Jacobians, and SIGKILL with pending work.
- 42 PostgreSQL tests, 6 cluster-store tests, and 2 Rust/Python process roundtrips passed.
- 42 Python tests passed, with one optional external-state test skipped and four
  negative-control subtests. A process sampler receives the full ragged domain,
  returns ragged samples and partial masses, and receives both feedback views.
- Strict Clippy checks passed for all targets with all features and with default
  features disabled. Rust/Python formatting and whitespace checks passed.

The Python runners share one coordinate codec for all four process roles. Fixed
coordinate widths use NumPy matrices; ragged blocks use tuples of writable 1D
arrays. Feedback transports a prefix table once and dense per-sample assignments,
so variable prefix lengths require no padding or repeated tuple payloads.

## Live comparisons

These results describe the prefix-model revision. The same dashboard address now
serves the [statistics-consistency revision](2026-10-22-statistics-consistency.md);
the earlier database and raw evidence remain preserved.

The [fresh dashboard](http://itphlies.tailb3264.ts.net:8097/) contains:

| Run | Example | Training attempts | Inference attempts |
| --- | --- | ---: | ---: |
| 1 | tetrahedron, ordinary | 131,072 | 131,072 |
| 2 | tetrahedron, stratified `(0,0)`, `(0,1)`, `(1,0)`, `(1,1)` | 131,072 | 131,072 |
| 3 | sine, ordinary | 33,554,432 | 268,435,456 |
| 4 | sine, stratified negative/positive | 33,554,432 | 268,435,456 |

Every run completed with produced and completed counts exactly equal to its budgets.
Ten persistent workers were reused between the pairs. Each arm initially had four
evaluators; idle workers accelerated the remaining sine arm after the other finished.
The earlier dashboard/database remains preserved on port 8096 with its original binary.

Sine integrates `2*pi*sin(2*pi*t)` over `[0,1]`, including the variable-change
Jacobian. The exact integral is zero.

| Estimator | Sine estimate | Reported standard error |
| --- | ---: | ---: |
| Ordinary | -0.0006154651466803 | 0.0002442090359900 |
| Stratified | -0.00000666117784065 | 0.00000938416665208 |

The reported error reduction is 26.02×. This reproduces the earlier high-precision
sine result to floating-point merging precision despite the new feedback API.

For tetrahedron, the exact vector integral is `[0,0,0]`.

| Component | Ordinary estimate ± reported error | Stratified estimate ± reported error |
| --- | --- | --- |
| x | 0.00273014 ± 0.00264122 | 0.000205864 ± 0.000596271 |
| y | 0.00184040 ± 0.00264337 | -0.00235314 ± 0.000615352 |
| z | -0.00148625 ± 0.00249454 | -0.000288254 ± 0.000635673 |

Browser checks opened all four completed inference tasks, verified the tuple-labelled
stratum counts, and recorded no page errors. Configuration and result data are in
[prefix-validation.json](../../benchmarks/stratification/prefix-validation.json).

## Observed uncertainty limitation

The tetrahedron y estimate is 3.82 **reported** standard errors from zero. We checked
32 matched-seed pairs through the production Symbolica/Havana path and
computed the live frozen proposal's variance independently by deterministic
quadrature. For each stratum and component, the latter evaluates
`(integral_cell f_j(t)^2/q_s(t) dt - I_sj^2) / n_s` using the saved piecewise-constant
Havana density. Tetrahedron cells are the four quarter-period intervals. Increasing
quadrature order from 64 to 128 changed the resulting errors by at most `1.2e-13`
relative, supporting numerical convergence of this independent calculation.

The resulting true conditional standard errors are approximately
`[0.0006062515, 0.0018542662, 0.0006436227]`. The sampled errors agree closely for
x and z, but the y error is only 33.2% of the independently calculated value.
The live y estimate is 1.27 true standard errors from zero. Rare high-weight
contributions explain why a single sampled variance can be optimistic at this budget.
This reproduces the tail behavior documented in the earlier single-axis benchmark.

Across 32 pairs, empirical/RMS-reported standard-deviation ratios for the stratified
arm were `[1.138, 0.805, 1.046]`; two-reported-error coverage was `[27,31,29]/32`.
These are small-ensemble diagnostics, not a guarantee of calibrated confidence
intervals. The minimum-attempt/match publication threshold does not establish that
importance-weight tails have been adequately sampled. No probability floor, error
inflation, or change to the estimator was introduced to conceal this limitation.

## Reproduction and evidence

Use the current [tetrahedron](../../resources/templates/runs/tetrahedron-stratified.toml)
and [sine](../../resources/templates/runs/sine-stratified.toml) cards, paired with
`tetrahedron-ordinary.toml` and `sine-ordinary.toml` in the same directory.
The in-process repeat experiment is:

```bash
cargo run --example stratification_ab -- \
  --trials 32 --training 131072 --inference 131072 --batch-size 2048 \
  --output tetrahedron-prefix-repetitions.json
```

The task's build logs, workflow evidence, raw results, 32-pair output, frozen grid,
quadrature script, and browser captures are retained on the execution host under
`/common/dev/cedric/setup-logs/stratification-prefixes-20261022/`.
A first CLI worker launch ended with the command executor; workers were relaunched
through the persistent server and the superseded launch record was marked failed.
