# Integrating an external evaluator

Use `EvaluatorDescriptor` plus `evaluate(..., request) -> EvaluationBatch`, or
implement the same [wire contract](protocol.md#selected-output-evaluator-contract)
in another language. The [Python observable example](examples/python_observables)
and [Rust GammaLoop wrapper](examples/rust_gammaloop) are complete providers.

## What the evaluator owns

For discrete label d and continuous coordinates x, GammaBoard estimates

`I = sum_d integral f(d,x) dx`.

The sampler owns the inverse joint sampling density, including discrete-choice
probabilities. Transforms own any additional sampling-coordinate Jacobians. The
evaluator returns f before those **outer** weights, including any parameterization
Jacobian implemented internally by the evaluator. GammaBoard multiplies by the
outer total weight once and owns masking, feedback projection and reduction.
Raw integrand/Jacobian `details` describe that internal mapping; the host does not
multiply by the reported Jacobian again. Observable bundles use the same ownership.
Each bundle describes one original draw; sum correlated fills in a bin before
squaring its contribution. Include draws with no fills.

Example: three discrete branches of unit continuous volume, with f=2 everywhere,
have integral 6. A uniform sampler chooses each branch with probability 1/3 and
returns weight 3. Return 2 from the evaluator, not 6 and not 2/3. With nonuniform
branch probabilities p(d), the weight includes 1/p(d); do not manually average
branches in the evaluator. Discrete-only leaves with zero continuous dimensions
are valid and use the same counting measure.

## Components and coordinates

The descriptor declares available component names. Array output columns follow
**request.components**, including its requested ordering and subset. A Python
mapping keyed by component name avoids manual reordering; the SDK selects columns.
Do not assume every request asks for all components or histograms. Unsupported
channels must fail explicitly. Descriptor capabilities do not change within a worker.

A branch receives exactly its declared continuous/discrete coordinates, using ragged
arrays and offsets where needed. GammaBoard does not pad shorter branches. If an
underlying library requires a fixed-width buffer, pad inside the adapter according
to that library's convention; those buffer entries are not new integration variables.
Declaring extra dummy coordinates changes the integration domain. Their integral is
one only when their measure has unit volume (or the integrand is normalized for it).
Missing physical coordinates must never be silently filled with zeros.

A pointwise constant C contributes `C * sum_d volume(domain_d)`. It is not one global
additive constant unless the total measure is one. For three unit-volume branches,
a global additive contribution C can be represented by C/3 on each branch. For
other domains derive the correct normalization, or keep the known analytic addition
outside the sampled result. Do not add an analytic term once per batch: that makes
results depend on batching.

## Check the executable

Create a JSON array of valid physical points, covering relevant discrete branches:

```json
[
  {"discrete": [], "continuous": [0.2]},
  {"discrete": [], "continuous": [0.4]},
  {"discrete": [], "continuous": [0.7]}
]
```

Then run (no GammaBoard instance or database is needed):

```bash
python process_api/check_evaluator.py \
  --command '["/absolute/path/evaluator"]' \
  --cwd /absolute/path/work \
  --init-args @/absolute/path/args.json \
  --points /absolute/path/points.json \
  --rtol 1e-10 --atol 1e-12 --json
```

The checker uses the executable's own runtime and dependencies. Python 3.11+,
NumPy and the SDK are required in the checker environment. Commands are arrays,
never shell strings. Domain discovery is automatic; `--domain @file` optionally
asserts it. Domain dimensions do not specify valid physical coordinate bounds, so
the checker deliberately requires your points instead of inventing a sampling box.
It starts fresh processes, evaluates points repeatedly and sends deliberate invalid
requests. Use disposable writable outputs in initialization arguments.

Checks cover individual versus batched calls (including empty batches), component
selection/reordering, JSON versus binary where both are advertised, invalid-request
rejection and recovery, and fresh-process replay. Counters/layouts must match exactly;
floating results use `abs(a-b) <= max(atol, rtol*max(abs(a),abs(b)))`. Diagnostic timings
must be finite and nonnegative but need not match. Unavailable channels are explicitly
skipped. Legacy fixed-output workers are reported inconclusive, not certified as
supporting the new contract.

Choose tolerances for your numerical precision and scales; defaults are not universal.
A deterministic evaluator should reproduce results within those tolerances across
batches and fresh processes. Seed any internal stochastic behavior explicitly.
`--reference known.json` checks expected full per-sample outputs for these points.
Without a reference, an evaluator that consistently applies a wrong normalization
can pass: this is a consistency checker, not a proof of the physics or integral.

`--timeout` bounds an RPC; `--time-limit` bounds the full check. Hung child processes
are terminated. Exit codes are 0 for pass, 1 for failed conformance, 2 for invalid
setup or an inconclusive required check, and 130 for interruption. JSON output lists
checks, skips, tolerances and the descriptor.

For a complete workflow test, run the same evaluator through a real instance with
known constant/vector integrals and nonuniform sampling. Verify the weighted result,
then retain the deployment or [import exported results](../docs/local-cli.md#inspecting-retained-or-imported-results)
for dashboard inspection. This additionally exercises the sampler, host reduction,
worker scheduling and database paths that the standalone checker does not test.
