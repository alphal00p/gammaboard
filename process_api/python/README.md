# GammaBoard process SDK

Python interfaces and runners for evaluators, samplers, materializers, and batch
transforms using `gammaboard-jsonrpc-v9`. Install with `pip install .` from this
directory, in the environment that contains your worker's scientific dependencies.

See the [process guide](../README.md) for setup and examples, the
[execution specification](../../docs/execution-model.md) for callback and recovery
semantics, and the [wire contract](../protocol.md) for non-Python implementations.

All callback constructors receive `domain: Domain` instead of separate dimension
arguments. Restore and stage handoff receive the same complete domain. Use
`domain.require_rectangular_shape()` only when your algorithm requires it; otherwise
inspect branching nodes, axis roles and rectangular `strata_dims`. Ragged blocks
are tuples of 1D arrays, while fixed-width blocks remain 2D arrays.

Sampler `feedback(feedback: Feedback)` receives exactly one original draw. Ordinary
samplers train on `feedback.values`, already masked by GammaBoard. Aware samplers
can also use `feedback.unmasked_values` and `feedback.strata.requested` / `.actual`
(prefix tuples). The optional `pdf` and `discrete_mass` methods return a number or
`None` per query. Exact complete-prefix masses are mandatory for stratified
inference; other marginals may remain unspecified. PDFs are normalized; there is
no `global_pdf_norm` callback.
