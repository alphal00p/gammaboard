# Sampling and evaluation execution model

This specifies the current GammaBoard pipeline, from generation through feedback
and durable recovery. It applies to native Rust implementations and process
adapters. The [wire protocol](../process_api/protocol.md) specifies message and
array encoding; the [process guide](../process_api/README.md) covers Python worker
setup. Configuration syntax is in [configuration](config.md).

## Ownership and flow

```mermaid
flowchart LR
    S[Sampler: private RNG and adaptation state] -->|draw: latent samples and feedback window| R[Sampler/aggregator: split and reserve]
    R -->|evaluation batches| Q[(PostgreSQL work queue)]
    Q -->|claim| M[Materializer]
    M --> T[Ordered batch transforms]
    T --> E[Evaluator and batch accumulator]
    E -->|accumulator and optional feedback| Q
    Q -->|ordered accepted results| A[Sampler/aggregator: merge and reassemble]
    A -->|one Feedback object per original draw| S
    A --> C[(Checkpoint and published results)]
```

The **sampler/aggregator worker** owns one active task's sampler, draw buffer,
feedback assembly, and cumulative accumulator. The sampler itself owns its RNG,
adaptation algorithm, training-window policy, and any private pending samples.

Each **evaluator worker** builds the effective task's materializer, transforms,
and evaluator. It claims independently executable batches and returns a fresh
batch accumulator plus feedback when requested. Several evaluator workers may
process fragments of the same draw concurrently.

The **control plane** activates tasks and places workers. A controller manages
ordinary child runs through its root worker pool. Campaign budget reservations
are durable control-plane state, not sampler-private counters.

The **database** holds definitions, claims, results, published snapshots, and
recovery checkpoints. Dashboard snapshots are observations; a saved checkpoint
is the boundary used to recover sampler execution.

For a sampling task, the execution sequence is:

1. Resolve the evaluator, sampler, accumulator, and transform sources. The first
   evaluator establishes the run domain; subsequent stages preserve that domain.
2. Obtain evaluator metadata and construct or restore the sampler. Metadata is
   JSON supplied by `Evaluator::metadata()`; it is not a sample or a result.
3. Save the initial checkpoint before producing work. A new sampling/plotting
   stage invalidates the previous live accumulator when it activates.
4. Request a bounded draw from `generate(max_samples)`. Retain its latent payload
   and feedback-window declaration, then split it into evaluator batches.
5. Queue each fragment with its task identity, resolved accumulator configuration,
   and `requires_training_values` flag. Evaluators claim work with ownership tokens.
6. Materialize each fragment, apply transforms in order, evaluate it, and validate
   the entire result before publishing the batch result.
7. Read accepted results in generation order, assemble feedback by original draw,
   call the sampler's feedback method, and merge batch accumulators.
8. Publish progress/results and check stopping conditions. Task completion and
   pause save the sampler, accumulator, queued-work boundary, and pending feedback.

## Runtime interfaces

The Rust contracts are [`Sampler`](../src/sampling/sampler.rs) and
[`Evaluator`, `Materializer`, `BatchTransform`](../src/evaluation/traits.rs).
Implementations are `Send`; their methods are called serially within an owning
worker. Concurrency comes from independent workers and queued batches.

| Interface | Required methods | Optional methods and defaults |
| --- | --- | --- |
| `Sampler` | `validate_domain(&Domain)`, `generate(usize) -> Generation`, `feedback(&Feedback)`, `snapshot() -> JSON` | `pdf_batch(points)` and `discrete_mass_batch(subspaces)` return unavailable entries; `persisted_output()` returns no output; `diagnostics()` returns `{}` |
| `Materializer` | `materialize_batch(&LatentBatch) -> Batch` | `validate_domain(&Domain)` succeeds by default |
| `BatchTransform` | `apply(Batch) -> Batch` | `validate_domain(&Domain)` succeeds by default |
| `Evaluator` | `domain() -> Domain`, `eval_batch(&Batch, &AccumulatorConfig, EvalBatchOptions) -> BatchResult` | `metadata()` and `diagnostics()` return `{}` |

Fallible methods return `Result<..., EngineError>`; metadata/diagnostics return
JSON directly. `SamplingConfig` owns construction identity: the `SamplerConfig`
algorithm, optional materializer, optional ordered transforms, and optional
stratification partition. Tasks refer to
it through the `sampler` field. The worker role is called `sampler_aggregator`
because the worker also accumulates completed results.

## Samples, domains, and queue payloads

A concrete [`Point`](../src/evaluation/batch.rs) has:

- `discrete: Vec<i64>`: the discrete branch/coordinate path;
- `continuous: Vec<f64>`: that branch's continuous coordinates;
- `weight_factors: Vec<{label, value}>`: factors whose product is the total weight.

`Point::new` records its supplied weight as `sampler_weight`. A concrete `Batch`
is an ordered list of points. Index `i` identifies the same logical sample through
materialization, transforms, evaluation, and feedback. Implementations must preserve
the sample count and order; filtering or reordering requires a different contract.

A [`Domain`](../src/utils/domain.rs) describes coordinate structure and axis roles,
not a proposal distribution, statistical independence, or integration bounds.

```json
{"rectangular":{"strata_dims":2,"discrete_cardinalities":[2,2,3],"continuous_dims":4}}
```

A rectangular node marks its first `strata_dims` discrete coordinates as strata;
the remainder are integrand coordinates. A branching node has `axis_role` equal to
`"stratum"` or `"integrand"`, an optional display `axis_label`, and ordered branches:
`{"discrete":{"axis_role":"stratum","axis_label":null,"branches":[{"index":0,"domain":...},...]}}`.
Branch labels are nonnegative, unique, increasing integers; gaps are allowed.
Roles default to integrand and rectangular `strata_dims` defaults to zero.

Strata are left-bound along each path: no stratum coordinate may follow an
integrand coordinate. Prefix lengths may differ between branches. For example,
`(0)`, `(1,0)`, `(1,1)`, `(1,3)` are valid complete stratum prefixes. A complete
prefix cannot also be a prefix of another stratum. Remaining integrand layouts
may also vary by branch.

Rust, the process protocol, and every Python callback receive the full domain.
Python uses typed `Domain`, `DiscreteDomain`, `RectangularDomain`, `DomainBranch`,
and `DiscreteAxisRole` objects. `domain.require_rectangular_shape()` is an explicit
convenience for algorithms requiring homogeneous contiguous coordinate arrays.
The shared SDK codec supports ragged rows: a fixed-width coordinate block is a
writable `(N,D)` NumPy array; a variable-width block is a tuple of writable 1D arrays.
Discrete and continuous blocks can independently be fixed or ragged. No padding,
truncation, or loss of branch labels occurs.

[`LatentBatch`](../src/sampling/latent_batch.rs) is the sampler-to-queue value:
`{nr_samples, payload}`. It need not contain fully materialized points. Current
payloads are:

| Payload kind | Contents and interpretation |
| --- | --- |
| `indexed_batch` | `discrete_signatures` is a dictionary of discrete coordinate rows; `discrete_map[i]` selects sample `i`'s row. `continuous_layouts[i]` gives its continuous width; `continuous_values` concatenates those rows. `weights[i]` is the sampling weight. All per-sample arrays have `nr_samples` entries, and the continuous widths sum to the flat coordinate length. |
| `havana_inference` | `rng_states` and `offset` describe replayable draws from a frozen Havana grid. RNG checkpoints bound replay at split points. The built-in materializer reconstructs coordinates and sampling weights using the stage's saved grid. Treat RNG state as implementation-owned serialized state. |

Process samplers return coordinate arrays on the wire; GammaBoard wraps them in
`indexed_batch` payloads. They cannot introduce arbitrary new latent payload kinds.

An `EvaluationBatch` wraps the latent samples and the resolved accumulator
configuration. Claim metadata supplies the batch/task identifiers and feedback
flag separately. Queue encoding is a versioned internal format; external workers
use the process protocol instead of decoding PostgreSQL queue bytes.

## Generation, splitting, and training windows

`generate(max_samples)` receives a **positive per-call upper bound**. It can change
between calls. A sampler returns exactly one of:

| Result | Meaning |
| --- | --- |
| `Batch { batch, training_remaining: None }` | Generate 1..`max_samples` inference samples; no feedback is requested for this draw. |
| `Batch { batch, training_remaining: Some(R) }` | Generate a feedback-bearing draw of size `N`, with `R >= N`. `R` counts the ungenerated samples in the current training window **before this draw**, including its `N` samples. |
| `Waiting` | No draw is available until outstanding feedback permits progress. It does not finish the task. |
| `Finished` | Generation is permanently finished for this sampler state. Existing batches and feedback drain before the task completes. |

An empty draw is invalid. `Waiting` is a feedback dependency, not a mechanism for
waiting indefinitely for an unrelated external event. A sampler with no possible
progress fails instead of silently finishing its task.

Three sizes are independent:

- **Draw size:** one `generate` result; limited by `max_generation_size` and budgets.
- **Evaluator batch size:** one queue item; controlled by fixed/adaptive queue tuning
  and `max_batch_size`. A draw can be consumed over several runtime ticks.
- **Training-window size:** sampler-owned adaptation boundary; it can span draws.

The runtime never changes the sampler's training policy. It uses the declared
window to size work and knows whether to request feedback. The sampler decides
when to return `Waiting`, consume pending feedback, update its model, or finish.

For example, a six-sample training window can produce two draws of three samples:
their `training_remaining` values are six and three. If the evaluator batch size
is two, each draw is split into fragments of two and one. Regardless of evaluation
completion order, the sampler receives two `feedback` calls, each with three
values, in original draw order. It does not receive one call per queue fragment
or one six-value call for the whole training window.

Private sample buffers belong in sampler state. The runtime tracks the split
cursor, remaining latent samples, and partially assembled feedback. Sampling
without feedback does not require storing private training buffers.

## Materialization and transforms

Materialization runs on evaluator workers **after** a queue fragment is claimed.
It maps one latent sample to one concrete point. An omitted materializer selects
the algorithm's built-in implementation: identity decoding for ordinary indexed
draws, or frozen-grid replay for Havana inference. A configured process
materializer overrides that choice.

Transforms run on the resulting concrete batch in configured order. They can
change coordinates and add Jacobian/importance factors, while preserving sample
identity, count, and a layout accepted by the run domain. A process transform
receives the current **total** weights and returns the replacement **total** weights;
it must include earlier factors in that result. GammaBoard does not multiply the
old total into the returned total again.

If a sample has sampling weight `w` and successive Jacobians `J1`, `J2`, the
evaluator-side total is `W = w * J1 * J2`. Process sampler/materializer/transform
weights must be finite and strictly positive. Signed integrands and signed
campaign coefficients are separate from those sampling weights.

For a normalized proposal density `p` relative to the desired integration measure,
the sampling weight is `1/p`; include volume and discrete-choice factors where
needed. The weight returned by generation must contain the complete sampling
correction. Native uniform sampling multiplies the cardinalities of every chosen
discrete branch/axis, including the stratum axis, into its inverse-density weight;
it integrates the discrete sum rather than the average over labels.
`pdf` returns an already normalized joint density in sampling coordinates.
`discrete_mass` returns exact normalized marginals. Either may return unavailable
entries; this never relaxes the requirement for complete generated weights.
For joint proposal `q(s,d,x) = p_s q(d,x|s)`, the conditional density is `q/p_s`.
After transforms, the total evaluator weight is `J/q`, not generally `1/q`.

Materializers and transforms have no snapshot/restore callback. Their output must
be reproducible from the supplied batch, immutable configuration, and immutable
referenced artifacts. Worker-local caches are allowed; correctness must not depend
on which worker receives a fragment, its partition size, or a previous invocation.
The same restriction applies to evaluator-private state that affects results.

Task-level `batch_transforms` overrides pipeline transforms. An omitted field
inherits the effective pipeline/preceding-stage transforms; an explicit empty
array clears them. See [task sources](config.md#task-sources-and-training).

## Evaluation, weights, and the return path

`EvalBatchOptions` contains `require_training_values`. A Rust evaluator returns:

```text
BatchResult {
    accumulator: AccumulatorState,
    feedback: Option<Feedback>
}
```

The accumulator is the result for **this batch**, not the run's previous state.
Feedback must be present exactly when requested, with one finite scalar per input
sample in input order. Its length is the claimed sample count, even for an
observable adapter whose internal integration counts differ.

`Feedback` contains `unmasked_values` and optional `strata`. The latter stores a
canonical table of complete prefix tuples and one requested/actual table index per
sample. Rust `requested_prefixes()` / `actual_prefixes()` and Python `.requested` /
`.actual` expose those tuples. The compact indices avoid repeating variable-length
prefixes for every sample. `Feedback.values()` in Rust / `.values` in Python is the
GammaBoard-owned masked view: misses become zero. Generic samplers consume this
view without understanding strata. An aware sampler can use the unmasked weighted
projection and actual prefix to improve training; it must account for the proposal
that generated the point, and any branch-dependent inverse maps/Jacobians. Relabeling
a point never changes its original sampling density or inference assignment.

Feedback is assembled in original draw order, including its prefix table and both
assignment arrays. Partial feedback survives checkpoint recovery. No sampler
awareness or invertibility flag changes runtime behavior.

### Ordinary values

An ordinary process callback returns unweighted values: one named component vector
per sample. For total weight `W_i` and unweighted component `f_ij`, the Rust adapter
accumulates `W_i * f_ij`. Feedback is `W_i * projection(f_i)`; a component projection
preserves the sign, and a norm projection computes the vector norm. The callback
must not multiply by the sampling weight itself.

`scalar` is statistical vector component `value`. Statistical vectors retain
moments for estimates/uncertainties and support adaptation feedback. `full_vector`
collects weighted component rows for inference/plotting; it has no training
projection. Use a statistical accumulator for feedback-bearing draws.

Invalid dimensions, non-finite values, or overflowing accumulator moments reject
the whole result. They do not silently remove samples, substitute zero, publish
partial accumulator updates, or feed the sampler an accepted partial result.

### GammaLoop/observable output

A GammaLoop process callback receives total weights and returns a
`GammaLoopBatchResult` containing native accumulator JSON and optional training
values. It applies the supplied weights **exactly once** to both state and
feedback. GammaBoard validates and merges the state; it does not reweight it.
When feedback is not requested, `training_values` must be absent/`None`.

Observable adapters can represent an independent native integration, such as a
one-trigger MadGraph calculation. Their internal accumulator counts can differ
from the number of queued input samples. Queue/task work accounting still counts
those input samples. This is why accumulator sample counts, task counts, and
whole-run counts need not be identical.

### Aggregation and feedback

Workers may finish fragments out of order. Ordered result ingestion and the
runtime's draw buffer restore the original feedback order. Each feedback-bearing draw
gets one complete feedback array per execution history. Batch accumulators merge
into the task's accumulator; progress and measurement projection derive from the
accepted history. A sampler can transform signed feedback internally for its
training algorithm, but the runtime does not replace it with absolute values.

## Sample budgets

A sample task's `max_samples` limits generation. Completed, queued, and buffered
samples already consume its remaining allocation. Error targets can stop new work
before that maximum; queued work may still drain. A generator's `Finished` result
can also end its task after outstanding work drains.

A campaign's `max_total_samples` caps logical samples across **all child stages**,
including training and plotting. Before workers generate samples, the controller
grants disjoint, durable child allowances whose sum cannot exceed the cap. Error-only
campaigns use the same grants without a global cap. Each grant is one child's
scheduling quantum, bounded by `allocation_window_samples`; pilot grants can be
smaller. The sampler subtracts previous-stage production and its current
produced/buffered work. Waiting for another grant is not sampler exhaustion.

A child retains its slot until that grant drains, independently of the other
children. Renewing a grant preserves the running sampler; replacing a child
checkpoints it through the existing worker handoff. Unfinished grants and live
placements identify incumbents after controller recovery, including a crash
between grant, placement and output publication. Retained children keep their
workers up to their balanced evaluator share. Resource shortages reduce active
concurrency according to the sampler count and the minimum evaluator allocation.
See [campaign allocation](config.md#controllers) for configuration.

An error target first prevents new grants while existing quanta drain. The target
is checked again on the resulting estimates, and completion is published only
after retiring workers release their placements and save their checkpoints.

Pilot grants and scheduling windows cannot bypass the cap. Completed child queues
return unused allowance; paused/active children retain reservations for buffered
or queued work. A cap too small to reach every child's final usable publishing
stage fails explicitly. Error convergence waits for the final publishing stages
and their progress publication.

The cap counts logical sample slots, not physical evaluator calls or CPU time.
Retries and recovery can execute a slot again. Observable adapters' internal
integration effort is also outside this input-sample count.

## Tasks, checkpoints, and failures

There are three distinct initialization paths:

| Path | State and counters |
| --- | --- |
| Fresh configuration | Construct a new sampler with its configured seed/arguments and the current evaluator metadata. |
| Supported stage handoff | Initialize a new task from an earlier saved sampler, for example Havana training → inference or a PDF diagnostic. Destination parameters apply; source work counters and pending work are not resumed. |
| Checkpoint recovery | Restore the exact saved sampler construction configuration and private state, accumulator, draw cursor/feedback fragments, queue cursors, and progress. |

Accumulator selection is independent of sampler handoff. Continuing an earlier
accumulator retains its statistics; an explicit accumulator configuration starts
fresh statistics. Task work counters count the new task's execution in either
case. Use a fresh inference accumulator when training samples should be excluded
from the reported estimate.

`SavedSampler` is `{version, config, state}`. `state` must be non-null JSON and must
contain all private RNG/adaptation state required by future `generate` and
`feedback` calls, including any draws already awaiting feedback. Construction
configuration is stored separately. `persisted_output()` is optional task output;
it is not a replacement for `snapshot()`.

Initial activation, graceful pause, and task completion save checkpoints. Recovery
validates the saved boundary, removes later speculative work, rolls back live
progress, and replays retained work consistently. Completed task/stage history is
separate from the live accumulator. A new publishing stage cannot reuse a previous
stage's live result as evidence that it has already sampled.

Claims and submissions use ownership tokens. An expired/superseded worker cannot
publish its late result as the current claim. Batch errors follow the queue retry
policy (`max_batch_retries`); persistent failures fail visibly. Callback errors
and native panics become task/batch failures rather than accepted numerical data.

Recovery preserves accepted history, not exactly-once external side effects.
Materialization, evaluation, and feedback after a checkpoint can be replayed.
External callbacks must tolerate replay; external sampler artifacts must remain
immutable, durable, and accessible wherever a resumed worker runs. Database
backups do not copy those files. See [operations](operations.md#backup-and-recovery)
for operator commands and [format compatibility](compatibility.md) for supported
state/protocol versions.

## Python interfaces and process boundaries

The SDK base classes are optional; matching methods are sufficient. Fresh classes
receive the complete typed `domain` and expanded `args` keywords.
Samplers additionally receive `evaluator_metadata`.
Pass a class to `run_*`, not an already constructed instance. `args` cannot define
the reserved keys `domain` or `evaluator_metadata`.

| Role | Python callback | Result |
| --- | --- | --- |
| Values evaluator | `eval(xs_discrete, xs_continuous)` | Unweighted `(N,)` for one component, `(N,C)`, or a mapping from every declared component name to an `(N,)` array |
| Observable evaluator | `eval_gammaloop(xs_discrete, xs_continuous, weights, *, require_training_values, stratification)` | `GammaLoopBatchResult(state, training_values, values)` |
| Sampler | `generate(max_samples)` | `SampleBatch(xs_discrete, xs_continuous, weights, training_remaining=None)` or `WAITING` / `FINISHED` |
| Sampler feedback | `feedback(feedback)` | Complete draw with masked `.values`, `.unmasked_values`, and optional `.strata` |
| Materializer | `materialize_batch(latent_batch)` | `Batch(xs_discrete, xs_continuous, weights)` |
| Transform | `transform_batch(xs_discrete, xs_continuous, weights)` | The same `Batch` type, with replacement total weights |

Fixed-width coordinates are NumPy arrays with shapes `(N,D)` and `(N,C)`;
variable-width coordinates are tuples of one-dimensional arrays. Weights and
feedback value arrays have shape `(N,)`. SDK callback input arrays own writable storage and
can be retained without corrupting later requests. All output arrays must have
the required shape and finite values. Component order comes from initialization.

Every sampler class must implement `snapshot()` and this restoration entry point:

```python
@classmethod
def from_snapshot(cls, *, snapshot, domain,
                  init_args, evaluator_metadata=None):
    ...  # Return a fully restored sampler instance.
```

`init_args` is the construction dictionary, not expanded keywords on restoration.
Optional SDK callbacks match the Rust defaults above. Python spellings are
`pdf(xs_discrete, xs_continuous)` and `discrete_mass(subspaces)`; the latter receives
a list of dictionaries mapping discrete dimension indices to fixed values.
Unavailable PDF or mass entries are `None`, independently for each query. Known
zero is `0.0`. Invalid numbers, including negative or non-finite densities and
masses outside `[0,1]`, are errors. There is no global PDF normalization hook.

Process roles use separate subprocesses and synchronous RPC calls; metadata flows
through GammaBoard, not a direct sampler-to-evaluator connection. Workers must
speak the current protocol. EOF closes a worker; the host waits its configured
grace period, then terminates the process group if necessary. Protocol framing,
role-specific methods, logging, and binary layouts are fully specified in the
[wire contract](../process_api/protocol.md).

## Scaled Voronoi stratification

This is a sampling-pipeline option for integration tasks; campaigns, scans and
tuning use it through their child integration definitions. It is not a run kind.
See [the configuration example](../resources/templates/runs/tetrahedron-stratified.toml).

`sampler.config.stratification` contains `kind = "voronoi"`, a stratum-only
`domain`, a `centers` map, and optional `scales`. The prefix domain is independent
of the map, contains zero continuous dimensions, and contains no integrand axes.
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
Native GammaLoop and process observable evaluators support a hybrid: the primary
real/imaginary integral uses stratified statistics, while histograms retain pooled
statistics. Their centres have two entries, `[real, imag]`; for a sign split on
the real part use `centers = { "0" = [-1, 0], "1" = [1, 0] }` with a two-choice prefix domain. Native scalar/vector value output continues
to use its declared component layout.

### Training statistics and the selected estimator

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
`W = J / (p_k q(x|k))`, where `k` identifies a complete prefix, inference accumulates
`Z_k = p_k W F(k,x)`. Per-stratum attempted counts, means and central cross moments
merge using parallel Welford updates. The final mean is the sum of stratum means;
its covariance is the sum of `sample_covariance_k / n_k`. Component errors are
square roots of its diagonal. Norm training projection remains a separate scalar
observable: its mean is the integral of the norm, not the norm of the vector mean.

### Pooled comparison during stratified inference

The primary estimate remains the stratified mean and its conditional error. The
expanded display also shows `basic_mean` and `basic_err`: the pooled estimator of
**the same masked, fully weighted draws**. This is a different mean, and does not
predict a separately trained unstratified sampler's performance.

No second accumulator is needed. For stratum count `n_s`, conditional mean `m_s`,
central moment sum `M2_s`, and frozen mass `p_s`, its pooled group has mean `m_s/p_s`
and central moment sum `M2_s/p_s²`. These groups are merged with the stable parallel
mean/covariance formula, including between-group mean differences. The resulting
pooled error uses `N*(N-1)`. A pooled comparison can be available while the primary
stratified error is still unavailable; it is never used as a fallback for stopping.

Measurements carry `estimator` (`pooled`, `stratified`, or `combined`), primary
`value`/`uncertainty`, and optional `basic: {mean,error}`. Controllers and tuning
continue to use only the selected primary estimate. Campaigns combine selected
child estimates and their uncertainties; any pooled comparison is combined
separately with the same coefficients. Relative errors and target comparisons
refer to the corresponding primary mean and error.

The same identity and comparison travel through history, JSON/CSV export, plots
and full-precision disclosures. Plot points retain the explicit primary error;
reconstructing it by subtracting rounded band endpoints can destroy small errors.
Missing errors are null/blank or labelled unavailable, never converted to zero.
The UI writes explicit same-scale parentheses, e.g. `1.5(0.5)`, and uses the same
shared exponent for the mean and parenthesized error.

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

For native histograms, all correlated events/cuts/counterterms from one original
sampler draw are grouped before squared bin contributions are accumulated. The
same classifier mask gates every histogram contribution, but the complete weight
`W` is retained: `H_b = sum_i W_i * mask_i * g_ib / N`. There is no `p_k` factor in
that pooled histogram estimator. A miss contributes no fills but still counts as
one attempted sample in every histogram, including underflow/overflow accounting.
Diagnostics count actual evaluations/events independently of this mask. Invalid
values still reject a batch even if its classifier would miss.

Histogram totals and the stratified primary estimate are different Monte Carlo
estimators and need not agree exactly. Histograms retain allocation noise. They
are labelled as pooled in the dashboard and are not rescaled or normalized to the
stratified result. Ratios involving both estimators require joint covariance that
this hybrid does not provide. The integral's per-stratum moments and pooled native
bundle are checkpointed and merged together; their attempted counts must agree.

The native adapter uses GammaLoop's single-sample evaluation entry points, gates
events before the one observable commit, and discards event records afterward.
This neither reevaluates the physics nor transports per-event data to the sampler.
Scalar/vector adapters share one accumulation path, and the evaluator runner
passes the optional partition through `EvalBatchOptions` to every adapter.

For Python observable callbacks, `stratification` is either `None` or a
`StratificationContext`. With a context, compute unweighted `(N,2)` real/imaginary
values, obtain `mask = stratification.mask(values)`, and apply it before committing
the native state. Training feedback remains **unmasked** and fully weighted. Return those **raw** values in
`GammaLoopBatchResult.values`; the host computes the conditional integral moments.
When no context is supplied, `values` must be `None`. The callback's state always
contains its existing pooled statistics, and counts all attempts when stratifying.
Adapters that launch an independent integration from one trigger, such as MadGraph
event generation, cannot satisfy this per-draw contract and reject stratification.

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

Task panels show attempts, matches, exact proposal probabilities, observed
allocations, component estimates, errors, and zero-hit warnings. Histogram estimates
retain an explicit pooled label, independently of the primary estimator. The
training projection is labelled separately, including means of per-sample norms. Pooled ESS, RSD and variance metrics are unavailable
for this estimator rather than reported with an inappropriate formula. Combined
means/errors feed the normal measurement and controller paths.

The coverage threshold does not guarantee that rare importance-weight tails have
been observed. At a finite budget, sampled variances can substantially underestimate
the proposal's true variance even when the estimator contract is satisfied. See the
[prefix-model validation](validation/2026-10-22-stratum-prefixes.md) for an observed
example and an independent frozen-proposal variance calculation.

### Stratum plot data and PDF probes

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
probed by this interface and fail explicitly. There is no sampler capability flag
or process protocol change. Native uniform sampling now returns its joint PDF on
rectangular and ragged discrete domains as well as purely continuous domains.

Display-only shape normalization uses trapezoid weights for the target on line
parameter `t` or plane parameters `(u,v)`. Native Havana line diagnostics also
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
view and counts preserve those cases. Diagnostic histograms are distributions of
finite raster values, not independent Monte Carlo observations, and carry no
Poisson or Monte Carlo error bars.

The shared panel serves ordinary and stratified runs, with one overview by
default and at most four stratum comparisons at a time. Comparable heatmaps share
both colour bounds in linear or logarithmic space. Scope, view, normalization,
zoom and colour settings are display-only. JSON/CSV export includes normalization
metadata, raw queried densities, exact native PDF intervals where available,
actual prefix assignments and diagnostic statuses.

Stratum histories use snapshot timestamps for elapsed time and the latest
preceding sampler performance report for the same task for reported sampler time.
Reports from other tasks or later timestamps cannot be substituted. Missing time
metadata remains absent, and merging a current frame cannot erase an existing
historical timestamp. Counts remain the canonical history key, independently of
the selected horizontal axis.
