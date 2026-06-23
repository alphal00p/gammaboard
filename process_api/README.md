# Process API Examples

This folder contains self-contained examples for external GammaBoard evaluators and samplers.

For the stable process protocol, see [../docs/process-runtime.md](../docs/process-runtime.md).

## Examples

- `examples/rust_breit_wigner_evaluator`: non-Python evaluator packaged with Apptainer. This direct protocol implementation demonstrates ragged row-major batches for inhomogeneous domains.
- `examples/python_scalar_sin`: NumPy scalar evaluator using a plain `.venv`.
- `examples/python_sampler_symbolica_havana`: Symbolica-backed Havana sampler using a Nix-pinned runtime.

## Python Package

The Python helpers live in `python/` and can be installed into any runtime:

```bash
pip install "gammaboard-process @ git+https://github.com/alphal00p/gammaboard.git@main#subdirectory=process_api/python"
```

Example entrypoint modules should be tiny:

```python
from demo_integrand import SinIntegrand
from gammaboard_process import run_evaluator

run_evaluator(SinIntegrand)
```

or:

```python
from demo_sampler import MySampler
from gammaboard_process import run_sampler

run_sampler(MySampler)
```

The `Evaluator` and `Sampler` ABCs are optional documentation/type-hint helpers.
`run_evaluator(...)` and `run_sampler(...)` accept any compatible class.

Evaluator classes implement:

```python
eval(xs_discrete, xs_continuous)
```

They may also implement `metadata()` or expose a `metadata` attribute returning
JSON-safe evaluator metadata. The default is `{}`.

Sampler classes implement:

```python
sample_plan()
produce_latent_batch(nr_samples)
ingest_training_values(training_values)
snapshot()
```

Samplers may also implement `training_samples_remaining()`,
`pdf(xs_discrete, xs_continuous)`, `discrete_pdf(subspaces)`, and
`get_diagnostics()`.

`discrete_pdf(subspaces)` receives a list of fixed-dimension maps:

```python
[{0: 2}]
```

It should return one marginal discrete PDF value per subspace, or `None` when
unsupported.

Fresh initialization always uses:

```python
ClassName(discrete_cardinalities=..., continuous_dims=..., **init_args)
```

If the sampler class accepts an `evaluator_metadata` keyword, the wrapper passes
the evaluator metadata to fresh and restored sampler construction.

Sampler restore may additionally implement:

```python
from_snapshot(*, snapshot, discrete_cardinalities, continuous_dims, init_args, evaluator_metadata=None)
```

These homogeneous dimensions are derived from the protocol `domain`; `init_args`
is the process config `args` table unchanged.
This wrapper shape is only a convenience layer. Non-Python runtimes can implement
the protocol directly. The bundled Python wrappers intentionally support only
homogeneous fixed-width batches and reject ragged offset layouts with a clear
error.
