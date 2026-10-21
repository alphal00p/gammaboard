# Stratification implementation plan

Status: proposed, before implementation. Prepared 2026-10-21.

## Agreed estimator and scope

Preserve the sampler's existing complete integration weights. Obtain exact,
normalized stratum probabilities through `discrete_pdf`, freeze the table for
inference, and derive conditional weights in GammaBoard. Quotas are optional
future allocation policy, not a prerequisite for the initial implementation.

For K user-supplied centres, define one new outer discrete axis k. All original
discrete/continuous coordinates remain underneath it. Given the evaluator's
unweighted component vector f(x), choose the nearest centre with deterministic
lowest-index tie breaking, and gate the integrand to F(k,x)=f(x) if it matches
k, otherwise zero. Classification happens before accumulation. Invalid evaluator
values still fail evaluation rather than being masked into valid zeros.

With q(k,x)=p_k q_k(x) and ordinary total weight W, inference accumulates
Z=p_k W F(k,x), including existing Jacobian factors exactly once. Every attempt
increments its requested stratum's count, including misses. Combine independent
stratum means by summation and mean variances by summation:

    estimate = sum_k mean(Z_k)
    covariance(estimate) = sum_k sample_covariance(Z_k) / n_k
    component_stderr[j] = sqrt(covariance(estimate)[j,j])

During adaptive training, preserve the full-weight feedback convention:
W * projection(F). Misses yield zero feedback in their original draw positions.
Do not use conditional-weight inference statistics to silently change the training
objective. Start inference with fresh statistics and the trained frozen proposal.

Initial support: ordinary scalar/vector outputs, including GammaLoop when using
its value output, native naive Monte Carlo, frozen Havana inference, and process
samplers that explicitly support the contract. Native aggregate observable output,
including one-trigger internal integrations, must reject stratified inference until
an adapter can gate and count its contributions before native accumulation. Raster,
correlated/QMC sampling, and adaptation during stratified inference are excluded
from the initial estimator. Ordinary existing execution remains available.

## Proposed configuration and API

Add immutable `stratification` partition settings to `SamplingConfig`, alongside
materialization/transforms, so the partition travels with `SavedSampler`. Fields:

- `centers`: nonempty list of distinct finite vectors; width matches the declared
  evaluator component vector.
- `scales`: optional finite positive per-component scales, defaulting to ones;
  distance is Euclidean after dividing components by these scales.
- Axis placement is fixed as the new outermost axis in v1; no arbitrary-axis
  configuration or independent-per-axis factorization is required.

Add `estimator = "pooled" | "stratified"` to sample tasks, default `pooled`.
Training uses the partition with pooled statistics and normally
`publish_result = false`; final inference explicitly selects `stratified`.
This separates partition/proposal identity from the choice of statistical estimator.

Illustrative additions to existing task cards:

```toml
[[task_queue]]
name = "train"
kind = "sample"
publish_result = false
estimator = "pooled"
stop_condition = { max_samples = 100000 }
accumulator = { config = "scalar" }
sampler = { config = { kind = "havana_training", stratification = { centers = [[-1.0], [1.0]] } } }

[[task_queue]]
name = "inference"
kind = "sample"
estimator = "stratified"
stop_condition = { max_samples = 1000000 }
accumulator = { config = "scalar" }
sampler = { config = { kind = "havana_inference" }, from_stage = "train" }
```

The inference handoff inherits the partition from the selected training snapshot.
An incompatible partition/component layout requires retraining; an explicit fresh
accumulator is required when changing to the stratified estimator. User-supplied
probability tables and empirical observed label fractions are not accepted as
substitutes for the sampler's distribution.

Retain `generate(max_samples)` and its complete `weights` arrays. Reuse
`discrete_pdf` with queries fixing only the new stratum coordinate. Its values
must represent true normalized marginal masses of the frozen sampling distribution,
including marginalization over the original discrete coordinates.

Add an explicit sampler declaration of independent, stationary inference sampling
(native implementation properties; process sampler initialization response). An
unknown declaration, unavailable PDF, nonpositive/nonfinite mass, or a table whose
sum is not one within a defined floating-point tolerance fails before generation.
The declaration is a contract, not a statistical test of a black-box sampler.
If full coverage requires a probability floor, it must be applied by the sampler
to its actual proposal and weights before querying the masses; the runtime must
never repair zero masses merely by modifying the reported probability table.

Persist a runtime-owned `StratificationPlan` containing the resolved partition,
component layout, masses, source-stage/proposal identity and plan ID. The plan ID
must not change merely because the sampler RNG advances. Probabilities are queried
from the final inference sampler after handoff, before generating its first draw,
and are not re-queried from a later sampler state during result ingestion.

## Implementation sequence

1. **Configuration, domains and contract**
   - Update `src/core/run_spec.rs`, `src/core/tasks.rs`, `src/utils/domain.rs`,
     `src/sampling/sampler.rs` and configuration validation.
   - Introduce typed partition/estimator/plan definitions and sampler properties.
   - Keep separate physical evaluator and lifted sampling domains. Original
     physical channel indices and evaluator metadata remain meaningful.
   - Define validation and handoff rules before adding execution paths.

2. **Training and value ingestion**
   - Add a shared classification/gating path in `src/evaluation/` used by native
     scalar/vector and process-value evaluators.
   - Materialize against the lifted sampling domain; retain the stratum label as
     runtime metadata while physical transforms/evaluators receive their original
     coordinates. Preserve labels through splitting and all count/order-preserving
     transforms; do not treat the new label as a GammaLoop physical channel.
   - Evaluate each proposal once, choose its centre, and gate before accumulation.
     Return full-length feedback arrays, including zeros, in original draw order.
   - Reject opaque aggregate observable output at validation, not after work starts.

3. **Frozen inference handoff and probabilities**
   - Extend `src/runners/stage_context.rs` and `src/sampling/sampler_aggregator/`.
   - Inherit the partition, restore the trained proposal, verify frozen independent
     sampling, query all K masses, and freeze the runtime plan.
   - Cover naive Monte Carlo and Havana first. Process samplers use the same
     probability queries and declare capabilities through the Python/Rust protocol.
   - Process samplers currently support checkpoint restoration but not stage
     handoff. Add an explicit optional `from_stage` SDK hook and distinct
     initialization `stage_source` payload, mutually exclusive with recovery
     `snapshot`. Transfer the trained model into fresh inference state without
     carrying training counters or pending feedback. Unsupported adapters fail
     explicitly; never reinterpret checkpoint recovery as an inference handoff.
   - Preserve complete generation weights; the runner owns conditional conversion.

4. **Stratified accumulator**
   - Add a dedicated typed accumulator state under `src/evaluation/accumulator/`.
   - Track attempted/matched counts and mergeable moments per stratum. Use stable
     online/parallel moment updates and component covariance for vector estimates.
   - Apply p_k to the full point weight exactly once for inference statistics;
     retain original full weights for existing training semantics and diagnostics.
   - Merge sufficient statistics by stratum, never average batch estimates or
     batch error bars. Reject merges with incompatible plan IDs/layouts.
   - No diagnostic bin cap or silent overflow is allowed for statistical strata.
   - n_k < 2 means uncertainty is unavailable, never zero. Require adequate pilot
     coverage before error-based stopping; surface low/zero-hit warnings without
     pretending that absence of hits proves a stratum's integral is zero.

5. **Queueing, recovery and task transitions**
   - Update `src/core/models.rs`, `src/core/checkpoint.rs`,
     `src/runners/sampler_aggregator.rs`, `src/runners/evaluator.rs`, and queue/result
     serialization to carry the immutable plan consistently.
   - Split/merge/delayed completion/retry must preserve label, plan identity, zero
     accounting and one-time accepted-result accounting.
   - Checkpoint the frozen plan and all per-stratum moments; restore them without
     querying mutable current distribution state. Retain existing budget semantics:
     misses consume logical sample slots and retries do not create extra slots.
   - Reject reuse of training statistics or a differently partitioned accumulator
     as the final stratified inference accumulator.
   - Bump persisted queue/checkpoint/model versions for incompatible representations;
     retain the project's fresh-database policy rather than add migration adapters.

6. **Process SDK, metrics, UI and documentation**
   - Update `process_api/protocol.md`, Python sampler initialization/capabilities,
     protocol fixtures and examples together. Revise the wire protocol to v7 for
     explicit stage handoff/capability semantics; update all in-repository producers
     and consumers together, with no old-version compatibility shim.
   - Ordinary evaluator value arrays and sampler full-weight arrays keep their
     meaning. Materializer/transform contracts preserve labels through the runtime.
   - Route combined estimates and stratified covariance into measurement and stopping
     logic. Make unsupported derived diagnostics explicit rather than reuse pooled
     formulas that no longer describe the estimator.
   - Show each stratum's attempts, matches, p_k, estimate and uncertainty, plus the
     combined result and coverage warnings in task output.
   - Document training/reset/frozen inference, the probability semantics, zero
     accounting, support boundaries and an executable scalar/complex example.

7. **Validation and acceptance**
   - Analytic normalization cases with unequal p_k, misses, signed cancellation,
     vector output, correlated original discrete coordinates and known integrals.
   - Statistical repeated-run tests compare empirical estimator covariance with
     reported covariance; include a poorly matched conditional sampler where
     stratification is not assumed to improve efficiency.
   - Invalid/zero/non-normalized/unavailable probabilities, unsupported samplers,
     changed partitions, nonfinite output, empty/undersampled strata and zero hits.
   - Vary generation/batch sizes, worker completion order and restart boundaries;
     verify identical accepted counts/moments and no duplication on retry.
   - End-to-end train -> fresh inference with native Havana and a process sampler,
     plus physical-transform/materializer and existing discrete-channel coverage.
   - Run appropriate Rust/Python/frontend suites and the PostgreSQL recovery suite;
     manually inspect the new task-output diagnostics in the webapp.

## Explicit follow-ups after the first implementation

- Native GammaLoop/other observable bundles: adapter-level classification before
  accumulation, including their own statistical counts and covariance semantics.
- Exact quotas and variance/cost-based allocation, while preserving the explicit
  sampling normalization contract.
- Learned centres, arbitrary classification projections, or adaptive inference:
  separate changes with their own estimator and recovery requirements.

The first code changes begin only after this plan has been presented for review.
