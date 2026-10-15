# External process workers

Use the Python SDK from the same revision as GammaBoard:

```bash
python -m pip install ./process_api/python
```

| Reference | Contents |
| --- | --- |
| [Execution model](../docs/execution-model.md) | Full sampler → materializer → transforms → evaluator → feedback contract, ownership, weights, budgets, and recovery |
| [Wire protocol](protocol.md) | JSON-RPC v6 framing, every role/method, binary arrays, and errors |
| [Python interfaces](python/src/gammaboard_process/abc.py) | Typed callbacks and their defaults |
| [Batch types](python/src/gammaboard_process/batches.py) | `Batch`, `SampleBatch`, `WAITING`, and `FINISHED` |

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

## Entrypoints

A worker module imports its class and calls the matching runner:

```python
from gammaboard_process import run_evaluator
from my_evaluator import MyEvaluator

if __name__ == "__main__":
    run_evaluator(MyEvaluator)
```

Use `run_sampler`, `run_materializer`, or `run_batch_transform` for the other roles.
The SDK owns framing and NumPy conversion; subclasses of the SDK base classes are
optional. The [Python interface table](../docs/execution-model.md#python-interfaces-and-process-boundaries)
defines constructor arguments, callback signatures, shapes, and restoration.

## Working examples

- [NumPy evaluator](examples/python_scalar_sin/README.md): ordinary scalar/vector values.
- [Havana sampler](examples/python_sampler_symbolica_havana/README.md): adaptive draws,
  feedback, snapshots, and inference.
- [GammaLoop observable evaluator](examples/python_gammaloop_observable/src/demo_evaluator.py):
  weighted native observable state and optional training values.
- [Rust/Apptainer evaluator](examples/rust_breit_wigner_evaluator/README.md): a direct
  wire implementation, including ragged coordinate layouts.

## Logs and lifecycle

Stdout belongs to the protocol. The SDK redirects `print` to logs; use
`gammaboard_process.log(message, level="info")` for structured levels.
`GAMMABOARD_LOG_LEVEL` filters worker messages; runtime `tracing.db_gammaboard_level`
filters persistence. Direct stderr is recorded at warning level.

A process can be restarted or receive replayed work. Keep numerical behavior
independent of prior evaluator/materializer/transform calls, and include all
sampler-private pending state in snapshots. See [failure and recovery semantics](../docs/execution-model.md#tasks-checkpoints-and-failures).
