# Process Runtime

GammaBoard can run evaluators, samplers, batch transforms, and materializers as ordinary child processes. A process can be Python, Rust, C++, or anything else that can read stdin and write stdout.

## Contract

The extension protocol is `gammaboard-jsonrpc-v1`.

- Transport: JSON-RPC 2.0 messages framed with `Content-Length` headers.
- Direction: GammaBoard sends requests on process stdin; the process writes responses on stdout.
- Logging: stderr is for logs. GammaBoard also tolerates limited accidental line-oriented stdout before a protocol frame and forwards it to logs, but stdout should be treated as reserved for framed responses.
- Concurrency: requests are synchronous. GammaBoard sends one request at a time per process and waits for the matching response id before sending the next.
- Batching: evaluator `eval_batch`, sampler `produce_latent_batch`, sampler `ingest_training_values`, sampler `pdf`, batch transform `transform_batch`, and materializer `materialize_batch` are batched.
- Arguments: run TOML `args = { ... }` is passed unchanged in `initialize`.
- Stability: adding optional fields is allowed; changing/removing fields or changing method semantics requires a new protocol string.

Frame shape:

```text
Content-Length: <UTF-8 JSON byte length>\r\n
\r\n
<JSON payload>
```

Every response must be a JSON object with:

```json
{ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } }
```

or:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32000,
    "message": "human-readable error",
    "data": { "traceback": "optional traceback or diagnostics" }
  }
}
```

GammaBoard requires exactly one of `result` or `error`.

## Evaluator Methods

`initialize` is sent once after process start:

```json
{
  "protocol": "gammaboard-jsonrpc-v1",
  "role": "evaluator",
  "domain": { "continuous": { "dims": 2 } },
  "components": ["value"],
  "observable": { "components": ["value"] },
  "args": {}
}
```

Return:

```json
{ "ok": true, "metadata": {} }
```

`metadata` is optional and defaults to `{}`. It may contain any JSON-safe
evaluator-derived information the sampler needs at initialization.

`eval_batch` evaluates a batch in ragged row-major form. The offset arrays have length `nr_samples + 1`; sample `i` uses `row_major[offsets[i]..offsets[i + 1]]`.

```json
{
  "nr_samples": 2,
  "components": ["value"],
  "xs_discrete_row_major": [0, 1, 1, 2],
  "xs_discrete_offsets": [0, 2, 4],
  "xs_continuous_row_major": [0.1, 0.2, 0.3, 0.4],
  "xs_continuous_offsets": [0, 2, 4]
}
```

Return `values_row_major` with length `nr_samples * len(components)` in sample-major, component-minor order:

```json
{ "values_row_major": [1.0, 2.0] }
```

Process evaluators should use a `kind = "vector"` accumulator with matching `components`. The configured vector training projection is also stored as a scalar aggregate and is used as sampler feedback.

## Sampler Methods

`initialize` is sent once after process start, and again after restore in a new process with `snapshot` populated:

```json
{
  "protocol": "gammaboard-jsonrpc-v1",
  "role": "sampler",
  "domain": { "continuous": { "dims": 2 } },
  "args": {},
  "snapshot": null,
  "evaluator_metadata": {}
}
```

Return:

```json
{ "ok": true }
```

`sample_plan` returns the sampler planning state:

```json
{ "plan": { "kind": "produce", "nr_samples": 8192 } }
```

Use `{ "plan": { "kind": "pause" } }` when the sampler cannot produce work yet.

`training_samples_remaining` returns either a non-negative integer or `null`:

```json
{ "remaining": 10000 }
```

`produce_latent_batch` returns row-major samples and positive finite weights:

```json
{ "nr_samples": 2 }
```

```json
{
  "xs_discrete_row_major": [],
  "xs_discrete_offsets": [0, 0, 0],
  "xs_continuous_row_major": [0.1, 0.2, 0.3, 0.4],
  "xs_continuous_offsets": [0, 2, 4],
  "weights": [1.0, 1.0]
}
```

`ingest_training_values` receives one projected training value per sample:

```json
{ "training_values": [0.7, 1.2] }
```

Return:

```json
{ "ok": true }
```

`pdf` probes the sampler PDF for many points at once:

```json
{
  "nr_samples": 2,
  "xs_discrete_row_major": [],
  "xs_discrete_offsets": [0, 0, 0],
  "xs_continuous_row_major": [0.1, 0.2, 0.3, 0.4],
  "xs_continuous_offsets": [0, 2, 4]
}
```

Return either an array of `f64 | null` values or `null` when unsupported:

```json
{ "values": [0.5, 0.25] }
```

`discrete_pdf` probes marginal PDFs for discrete subspaces:

```json
{
  "subspaces": [
    { "fixed_dims": [{ "dim": 0, "value": 2 }] }
  ]
}
```

Return either an array of `f64 | null` values or `null` when unsupported:

```json
{ "values": [0.125] }
```

`snapshot` returns any JSON-safe state needed to restore the sampler:

```json
{ "snapshot": { "grid": "...", "seed": 42 } }
```

`get_diagnostics` returns optional JSON-safe diagnostics:

```json
{ "diagnostics": { "training_rate": 0.01 } }
```

## Batch Transform Methods

`initialize` is sent once after process start:

```json
{
  "protocol": "gammaboard-jsonrpc-v1",
  "role": "batch_transform",
  "domain": { "continuous": { "dims": 2 } },
  "args": {}
}
```

Return:

```json
{ "ok": true }
```

`transform_batch` maps one concrete batch to another concrete batch after
materialization and before evaluation. This is the recommended process-API hook
for parametrizations that operate on sampled coordinates.

```json
{
  "nr_samples": 2,
  "xs_discrete_row_major": [],
  "xs_discrete_offsets": [0, 0, 0],
  "xs_continuous_row_major": [0.1, 0.2, 0.3, 0.4],
  "xs_continuous_offsets": [0, 2, 4],
  "weights": [1.0, 1.0]
}
```

Return the transformed concrete points in the same row-major form:

```json
{
  "xs_discrete_row_major": [],
  "xs_discrete_offsets": [0, 0, 0],
  "xs_continuous_row_major": [0.2, 0.4, 0.6, 0.8],
  "xs_continuous_offsets": [0, 2, 4],
  "weights": [2.0, 2.0]
}
```

Weights must be positive finite `f64` values. Continuous coordinates must be
finite. The transformed batch is validated against the run domain before
evaluation.

## Materializer Methods

`initialize` is sent once after process start:

```json
{
  "protocol": "gammaboard-jsonrpc-v1",
  "role": "materializer",
  "domain": { "continuous": { "dims": 2 } },
  "args": {}
}
```

Return:

```json
{ "ok": true }
```

`materialize_batch` converts one queued latent batch into concrete evaluator
points. `latent_batch` is the JSON form stored by Gammaboard; for current
samplers this is usually an `indexed_batch` payload containing discrete
signatures, per-sample discrete-map entries, continuous layouts/values, and
weights.

```json
{
  "nr_samples": 2,
  "latent_batch": {
    "nr_samples": 2,
    "accumulator": { "kind": "scalar" },
    "payload": {
      "kind": "indexed_batch",
      "discrete_signatures": [[]],
      "discrete_map": [0, 0],
      "continuous_layouts": [2, 2],
      "continuous_values": [0.1, 0.2, 0.3, 0.4],
      "weights": [1.0, 1.0]
    }
  }
}
```

Return concrete points in the same ragged row-major form used by sampler output:

```json
{
  "xs_discrete_row_major": [],
  "xs_discrete_offsets": [0, 0, 0],
  "xs_continuous_row_major": [0.1, 0.2, 0.3, 0.4],
  "xs_continuous_offsets": [0, 2, 4],
  "weights": [1.0, 1.0]
}
```

Weights must be positive finite `f64` values. Continuous coordinates must be
finite. The materialized batch is validated against the run domain before
evaluation.

## Config Shape

The process command is explicit in run config. GammaBoard does not append worker scripts or assume Python.

```toml
kind = "process_evaluator"
command = ["python", "-m", "my_runtime.evaluator_worker"]
cwd = "$resources"
domain = { continuous = { dims = 2 } }
components = ["value"]
args = { scale = 1.0 }
```

Process batch transforms are task-level stage state:

```toml
[[task_queue.batch_transforms]]
kind = "process_batch_transform"
command = ["python", "-m", "my_runtime.transform_worker"]
cwd = "$resources"
args = { scale = 1.0 }
```

Process materializers are attached to sampler configs:

```toml
[task_queue.sampler_aggregator.config]
kind = "process_sampler"
command = ["python", "-m", "my_runtime.sampler_worker"]
cwd = "$resources"

[task_queue.sampler_aggregator.config.materializer]
kind = "process_materializer"
command = ["python", "-m", "my_runtime.materializer_worker"]
cwd = "$resources"
args = { scale = 1.0 }
```

The protocol uses `domain` as the authoritative coordinate layout. Homogeneous wrappers may derive fixed dimensions from it internally, but run config should not define separate shape hints.

Domain variants are snake_case. `rectangular` is the compact form for fixed-cardinality discrete grids:

```toml
domain = { rectangular = { discrete_cardinalities = [2, 3], continuous_dims = 2 } }
```

`command` is literal argv after `$resources` expansion. `cwd` defaults to `$resources`.
GammaBoard does not infer paths, append worker scripts, or inject Apptainer binds.
For Apptainer, spell out binds explicitly, for example `--bind`, `$resources:$resources`.
`args` is protocol payload and is passed through unchanged.
Nix, Apptainer, virtualenvs, and system packages are all just ways to make this command available.

## Python Package

The Python helpers live in `process_api/python` and can be installed into a runtime:

```bash
pip install "gammaboard-process @ git+https://github.com/alphal00p/gammaboard.git@main#subdirectory=process_api/python"
```

Worker modules are ordinary Python entrypoints:

```python
from demo_integrand import SinIntegrand
from gammaboard_process import run_evaluator

run_evaluator(SinIntegrand)
```

```python
from demo_sampler import SymbolicaHavanaSampler
from gammaboard_process import run_sampler

run_sampler(SymbolicaHavanaSampler)
```

```python
from demo_materializer import MyMaterializer
from gammaboard_process import run_materializer

run_materializer(MyMaterializer)
```

```python
from demo_transform import MyTransform
from gammaboard_process import run_batch_transform

run_batch_transform(MyTransform)
```

The `Evaluator`, `Sampler`, `BatchTransform`, and `Materializer` ABCs are optional documentation/type-hint helpers.
Inheritance is not required; `run_evaluator(...)`, `run_sampler(...)`,
`run_batch_transform(...)`, and `run_materializer(...)` accept any compatible class.

Evaluator classes implement `eval(xs_discrete, xs_continuous)`.

Sampler classes implement `sample_plan`, `produce_latent_batch`, `ingest_training_values`, `snapshot`, and optional `training_samples_remaining` / `pdf` / `discrete_pdf` / `get_diagnostics`.

Batch transform classes implement `transform_batch(xs_discrete, xs_continuous, weights)` and return a
`TransformedBatch`, a dict with `xs_discrete` / `xs_continuous` / `weights`, a
3-tuple, or any object with those attributes.

Materializer classes implement `materialize_batch(latent_batch)` and return a
`MaterializedBatch`, a dict with `xs_discrete` / `xs_continuous` / `weights`, a
3-tuple, or any object with those attributes.

Fresh initialization uses
`ClassName(discrete_cardinalities=..., continuous_dims=..., **args)`.
Sampler restore may instead implement
`from_snapshot(snapshot=..., discrete_cardinalities=..., continuous_dims=..., init_args=...)`.

## Benchmark

Run the ignored protocol benchmark with:

```bash
cargo test -q process_evaluator_eval_batch_protocol_benchmark -- --ignored --nocapture
```

The benchmark uses a tiny Python echo evaluator and measures real `eval_batch` framing overhead for small, medium, and large batches.
