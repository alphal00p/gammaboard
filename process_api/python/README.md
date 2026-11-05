# GammaBoard process SDK

Python interfaces and runners for evaluators, samplers, materializers, and batch
transforms using `gammaboard-jsonrpc-v10`. Install with `pip install .` from this
directory, in the environment that contains your worker's scientific dependencies.

See the [process guide](../README.md) for setup and examples, the
[execution specification](../../docs/execution-model.md) for callback and recovery
semantics, and the [wire contract](../protocol.md) for non-Python implementations.

Evaluator `eval(discrete, continuous)` returns unweighted `(N,C)` values (or `(N,)`
for a scalar), a component-name mapping, or `ObservableBatch` with one native
observable bundle per input. It receives integrand coordinates, not sampling
weights or stratum prefixes. Correlated fills belong to their original sample;
GammaBoard applies weights, masks and statistical reduction.

Sampler `generate(max_samples)` returns weighted `SampleBatch` draws, `WAITING`
for pending feedback, or `FINISHED`. Its `snapshot`/`from_snapshot` preserve private
RNG, adaptation and pending draws; `from_stage` optionally starts a fresh stage
from a trained proposal. The sampler does not aggregate published observables.

All callback constructors receive `domain: Domain`. Samplers/materializers see the
augmented domain; evaluators/transforms see the integrand domain. Use
`domain.require_rectangular_shape()` only when your algorithm requires it; otherwise
inspect branching nodes, axis roles and rectangular `strata_dims`. Ragged blocks
are tuples of 1D arrays, while fixed-width blocks remain 2D arrays.

Sampler `feedback(feedback: Feedback)` receives exactly one original draw. Ordinary
samplers train on `feedback.values`, already weighted and masked by GammaBoard.
Aware samplers can use `feedback.unmasked_values` and `feedback.strata.requested` / `.actual`
(prefix tuples). The optional `pdf` and `discrete_mass` methods return a number or
`None` per query. Exact complete-prefix masses are mandatory for stratified
inference; other marginals may remain unspecified. `pdf` is a normalized joint
density; `discrete_mass` is a normalized marginal probability. Neither includes
transform Jacobians.
