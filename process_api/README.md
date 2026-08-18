# Process API Examples

This folder contains self-contained examples for external GammaBoard evaluators, samplers, batch transforms, and materializers.

For the stable process protocol, see [../docs/process-runtime.md](../docs/process-runtime.md).

## Examples

- `examples/rust_breit_wigner_evaluator`: non-Python evaluator packaged with Apptainer. This direct protocol implementation demonstrates ragged row-major batches for inhomogeneous domains.
- `examples/python_scalar_sin`: NumPy scalar evaluator using a plain `.venv`.
- `examples/python_sampler_symbolica_havana`: Symbolica-backed Havana sampler using a Nix-pinned runtime.

## Python Package

The Python helpers live in `python/` and can be installed into any runtime:

```bash
pip install "gammaboard-process @ git+https://github.com/alphal00p/gammaboard.git@fdd59328814019a524a7838783efde8b42af3d50#subdirectory=process_api/python"
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

or:

```python
from demo_materializer import MyMaterializer
from gammaboard_process import run_materializer

run_materializer(MyMaterializer)
```

or:

```python
from demo_transform import MyTransform
from gammaboard_process import run_batch_transform

run_batch_transform(MyTransform)
```

The `Evaluator`, `Sampler`, `BatchTransform`, and `Materializer` ABCs are optional documentation/type-hint helpers.
`run_evaluator(...)`, `run_sampler(...)`, `run_batch_transform(...)`, and
`run_materializer(...)` accept any compatible class.

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

Batch transform classes implement:

```python
transform_batch(xs_discrete, xs_continuous, weights)
```

They return a `TransformedBatch`, a dict with `xs_discrete` /
`xs_continuous` / `weights`, a 3-tuple, or any object with those attributes.
Use them in task configs with `batch_transforms = [{ kind = "process_batch_transform", ... }]`.

Materializer classes implement:

```python
materialize_batch(latent_batch)
```

They return a `MaterializedBatch`, a dict with `xs_discrete` /
`xs_continuous` / `weights`, a 3-tuple, or any object with those attributes.
Attach them to sampler configs with `materializer = { kind = "process_materializer", ... }`.

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
homogeneous fixed-width evaluator/sampler/batch-transform/materializer batches
and reject ragged offset layouts with a clear error.

## Logging

The protocol owns stdout, so logs go over stderr. The package handles this:

```python
import gammaboard_process as gb

gb.log("started")                  # default info
gb.log("unstable", level="warn")
gb.log(f"loss={loss}", level="debug")
print("captured at info")          # print() is rerouted to info
```

`gb.log(message, level=...)` takes `trace|debug|info|warn|error` and records a
runtime log with `source = "worker"` at that level. Direct stderr writes
(tracebacks, native libraries) are recorded unstructured at `warn`. Messages
below `GAMMABOARD_LOG_LEVEL` (env, default `info`) are dropped in the worker; the
server then keeps only those at or above `db_gammaboard_level` (default `info`).
See [../docs/process-runtime.md](../docs/process-runtime.md) for the wire format
non-Python workers use.
