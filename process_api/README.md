# Process API Examples

This folder contains self-contained examples for external GammaBoard evaluators and samplers.

For the stable process protocol, see [../docs/process-runtime.md](../docs/process-runtime.md).

## Examples

- `examples/python_scalar_sin`: NumPy scalar evaluator packaged with Nix.
- `examples/python_sampler_symbolica_havana`: Symbolica-backed Havana sampler packaged with Nix and a pinned Symbolica wheel.
- `examples/rust_breit_wigner_evaluator`: non-Python evaluator packaged with Apptainer. This is the simplest direct protocol implementation.

## Python Wrapper Pattern

The Python examples include worker scripts that implement `gammaboard-jsonrpc-v1` and then load a normal Python class from `args.module` and `args.class`.

Evaluator classes implement:

```python
eval(xs_discrete, xs_continuous)
```

Sampler classes implement:

```python
sample_plan()
training_samples_remaining()
produce_latent_batch(nr_samples)
ingest_training_values(training_values)
snapshot()
```

Samplers may also implement `pdf(xs_discrete, xs_continuous)` and `get_diagnostics()`.

Optional constructors:

```python
from_config(discrete_cardinalities, continuous_dims, init_args)
from_snapshot(snapshot, discrete_cardinalities, continuous_dims, init_args)
```

This wrapper shape is only a convenience layer. Non-Python runtimes can implement the protocol directly.
