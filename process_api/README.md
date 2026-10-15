# External evaluators and samplers

Install the Python SDK from the same GammaBoard revision as the server:

```bash
pip install ./process_api/python
```

Use **gammaboard-jsonrpc-v6** for every worker. Non-Python implementations use the
[wire contract](protocol.md). Examples: [NumPy evaluator](examples/python_scalar_sin/README.md),
[Havana sampler](examples/python_sampler_symbolica_havana/README.md),
[Rust/Apptainer evaluator](examples/rust_breit_wigner_evaluator/README.md), and
[GammaLoop observable evaluator](examples/python_gammaloop_observable/run.toml).

## Configure a worker

```toml
[evaluator]
kind = "process_evaluator"
command = ["/path/to/venv/bin/python", "-m", "my_worker"]
cwd = "/shared/project"
domain = { rectangular = { discrete_cardinalities = [], continuous_dims = 2 } }
output = { kind = "values", components = ["value"] }
args = { scale = 2.0 }
```

All process roles share `command`, optional `cwd`, `args`, and
`shutdown_grace_seconds` (30 by default). Process samplers inherit the run domain.
Ordinary evaluator output defaults to one `value` component; tasks choose
statistical or full-vector accumulation with matching components. Use
`output = { kind = "gammaloop" }` for native observable bundles.
Commands may launch a local executable or an explicit container invocation.

## Python callbacks

Entrypoints call `run_evaluator(MyClass)`, `run_sampler(MyClass)`,
`run_materializer(MyClass)`, or `run_batch_transform(MyClass)`. Inheritance from
SDK base classes is optional. The [typed interfaces](python/src/gammaboard_process/abc.py)
and [batch types](python/src/gammaboard_process/batches.py) are the reference.

Fresh constructors receive `discrete_cardinalities`, `continuous_dims`, and
expanded `args`; samplers also receive `evaluator_metadata`. Python wrappers
support homogeneous fixed-width layouts. Callback inputs own writable NumPy
storage and may be retained without affecting later requests.

| Role | Callback and result |
| --- | --- |
| Evaluator | `eval(xs_discrete, xs_continuous)` → unweighted array or component-name mapping |
| Observable evaluator | `eval_gammaloop(xs_discrete, xs_continuous, weights, *, require_training_values)` → `GammaLoopBatchResult` |
| Sampler | `generate(max_samples)` → `SampleBatch`, `WAITING`, or `FINISHED` |
| Materializer | `materialize_batch(latent_batch)` → `Batch` |
| Transform | `transform_batch(xs_discrete, xs_continuous, weights)` → `Batch` |

Rust applies weights/projection for ordinary values. Observable callbacks apply
supplied weights exactly once to state and requested feedback. Invalid values,
shapes, or overflowing moments reject the whole batch. JSON must be finite.
`metadata()` and `diagnostics()` are methods returning JSON-safe data.

## Sampler feedback and restoration

A draw contains 1..`max_samples` samples. `training_remaining` is `None` for
inference or the remaining training-window size **including this draw**. It must
be at least the draw size. The runtime splits draws into evaluator batches and
calls `feedback(values)` once per feedback-bearing draw, in original generation
and sample order, with one already-weighted scalar per sample.

Draws, evaluator batches, and training windows are independent. `WAITING` waits
for outstanding feedback; the sampler decides when to update its model.
`FINISHED` stops generation while pending work drains. Private training buffers
belong to the sampler; the runtime keeps split cursors and partial feedback.

`snapshot()` must return non-null JSON containing all private state needed for
pending feedback. Restoration requires:

```python
@classmethod
def from_snapshot(cls, *, snapshot, discrete_cardinalities, continuous_dims,
                  init_args, evaluator_metadata=None): ...
```

Construction config is stored separately by GammaBoard. External files must be
immutable, durable, and available to resumed workers; snapshots do not copy them.
Optional sampler callbacks: `pdf`/`discrete_pdf` (default `None`), `global_pdf_norm`
(default `1.0`), `persisted_output` (default `None`), and `diagnostics` (default `{}`).

## Logging and shutdown

Stdout belongs to the protocol. The SDK redirects `print` to logs; use
`gammaboard_process.log(message, level="info")` for structured levels.
`GAMMABOARD_LOG_LEVEL` filters worker messages; runtime `tracing.db_gammaboard_level`
filters persistence. Direct stderr is recorded at warning level.
On shutdown GammaBoard closes stdin, waits the grace period, then sends SIGTERM
and, after five seconds, SIGKILL to the process group if needed.
