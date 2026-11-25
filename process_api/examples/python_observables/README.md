# Synthetic observable evaluator

This is a protocol tutorial, **not a GammaLoop amplitude**. It evaluates
`f(x) = 6*x*(1-x)` on `[0,1]`, whose integral is one, and returns one unweighted
GammaLoop-format histogram bundle per input point.

The Python worker needs NumPy and the SDK; see [shared setup](../README.md).
GammaBoard currently needs its `gammaloop` build feature to reduce and display
these bundles, even though no generated physics state is needed.

- [Ordinary run](run.toml): uniform sampling and an x histogram.
- [Stratified run](run-stratified.toml): Havana training, frozen inference, and
  pooled versus stratified histogram estimates.

Set the card's `python` replacement to the worker interpreter, then create the
chosen card from the repository root:

```bash
./gammaboard run create process_api/examples/python_observables/run-stratified.toml
```

The two Voronoi centres partition the **integrand values**, not x directly.
GammaBoard owns integration weights, masking, zero accounting and aggregation.

The descriptor declares `real`/`imag` values, histograms and a real scalar projection.
The worker computes histograms only for requests with `observables=True`. Both cards
discover the domain and capabilities automatically; the ordinary card also plots
values from the same evaluator without generating histograms. Diagnostics are
unavailable for this synthetic example, rather than reported as zero measurements.
