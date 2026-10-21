# Stratification validation — 2026-10-21

Implemented scaled output-space Voronoi partitioning as a sampling-pipeline
option. Training uses full-weight feedback with zeros for misses; frozen inference
uses exact label probabilities and independent per-stratum moments/covariance.
The runtime preserves the original physical domain for evaluators/transforms,
keeps stratum identity in queued work, and restores the immutable probability plan
with checkpoints. No statistical-guarantee capability flags were added.

The source branch is `stratification`. Database model 3, checkpoint/saved sampler
model 2, queue GBQ2 and process protocol v7 deliberately require matching current
components. Existing databases/demos were preserved; validation used fresh databases.

## Checks

- 362 library tests passed, followed by the additional all-miss convergence guard
  regression. One pre-existing library test remains intentionally ignored.
- 42 PostgreSQL integration tests pass across the full run and corrected
  checkpoint-fixture rerun. Fixtures now use the current version constants.
- 35 backend workflow tests passed, including both new stratification cases,
  campaigns, scans/tuning, lifecycle, and seeded recovery invariants.
- Native Havana training -> stratified inference -> pause/checkpoint -> resume
  preserves proposal identity and exactly 16,384 inference attempts.
- A process sampler's explicit fresh-stage handoff retains its learned probability
  table `[0.3,0.7]` while resetting stage counters; distributed inference is normalized.
- 36 Python SDK/integration tests passed, with one existing skip. Both Rust process
  protocol roundtrip tests passed, covering ordinary/observable output and the
  materializer/transform interfaces.
- Strict all-target/all-feature Clippy, no-default-feature compilation, Rust
  formatting, and changed-file whitespace checks passed.
- Both distributed tetrahedron run cards pass CLI validation including runtime
  probes. Unsupported aggregate observables, invalid scales and incomplete
  probability tables fail explicitly.

One initial broad test run encountered `ETXTBSY` because another build replaced
the CLI while tests were spawning it; rerunning with a pinned executable passed.
The new process test initially picked up a different Python's NumPy through the
Nix shell's module path; isolation of its Python path resolved the environment
failure. Neither failure was suppressed or changed into a passing test.

## Application check

Started a separate local deployment on frontend 8096/API 4026 using the existing
PostgreSQL server with a new database. Executed both tetrahedron cards, assigned
the ordinary run through the browser, and inspected completed stratified inference.
Per-stratum attempts/matches, probabilities and all component estimates rendered
without page errors. Review found and fixed persisted-history decoding and the
sampler-domain display so the additional sampling axis is represented correctly.

## Statistical experiment

The production-component A/B executable ran 128 matched pairs at 131,072 training
and 131,072 inference attempts per arm. Observed variance reductions were 15.42×,
4.91×, 13.76× for x/y/z. A 64-pair follow-up increased inference eightfold and
obtained 20.71×, 3.70×, 10.91×. All attempts, including misses, count toward budgets.

The y component showed rare importance-weight tails: nominal 95% coverage was
88.28% in the primary experiment and 92.19% in the larger-budget follow-up. The
latter's y pull standard deviation improved from 1.26 to 1.06. These results
support variance reduction without promising reliable Gaussian intervals at every
finite budget. No normalization discrepancy was found in the deterministic
unequal-probability tests or the independent repeated-sampling calibration test.

Raw results, reproducible commands, plots, bootstrap intervals and all caveats are
in the [A/B report](../../benchmarks/stratification/README.md).
