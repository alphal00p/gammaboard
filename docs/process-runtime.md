# Process Runtime

GammaBoard can run evaluators, samplers, batch transforms, and materializers as ordinary child processes. A process can be Python, Rust, C++, or anything else that can read stdin and write stdout.

## Contract

The extension protocol is `gammaboard-jsonrpc-v4`.

- Transport: JSON-RPC 2.0 messages framed with `Content-Length` headers, plus an
  optional raw binary block (see "Binary payloads" below).
- Direction: GammaBoard sends requests on process stdin; the process writes responses on stdout.
- Logging: stderr is for logs (stdout is reserved for framed responses; GammaBoard tolerates only limited accidental line-oriented stdout before a frame). Worker stderr is recorded as runtime logs with `source = "worker"`. A line of the form `@gblog\t<level>\t<message>` (level ∈ trace,debug,info,warn,error) is emitted at that level; any other stderr line is recorded at `warn`. Both are then filtered by runtime config `tracing.db_gammaboard_level` (default `info`).
- Concurrency: requests are synchronous. GammaBoard sends one request at a time per process and waits for the matching response id before sending the next.
- Batching: evaluator `eval_batch`, sampler `generate`, sampler `feedback`, sampler `pdf`, batch transform `transform_batch`, and materializer `materialize_batch` are batched.
- Arguments: run TOML `args = { ... }` is passed unchanged in `initialize`.
- Stability: adding optional fields is allowed; changing/removing fields or changing method semantics requires a new protocol string.

## Lifecycle

On Unix, GammaBoard starts each process runtime in an isolated process group, so
terminal signals sent to GammaBoard do not interrupt external workers directly.
Shutdown is owned by GammaBoard: it stops issuing requests, closes the worker's
stdin, and waits for the process to exit on EOF. If it remains alive for
`shutdown_grace_seconds` (default `30`), GammaBoard sends `SIGTERM` to the
process group, waits five more seconds, and then uses `SIGKILL` as a final
fallback. `shutdown_grace_seconds` is an optional field on every
`process_evaluator`, `process_sampler`, `process_batch_transform`, and
`process_materializer` config.

Requests remain synchronous. A request already being handled is allowed to
return before the owning runner begins this shutdown sequence, subject to the
normal request timeout.

Frame shape:

```text
Content-Length: <UTF-8 JSON byte length>\r\n
Binary-Length: <binary byte length>\r\n      (optional; absent means 0)
\r\n
<JSON payload><binary payload>
```

Binary payloads: large numeric arrays are carried as a raw little-endian binary
block appended after the JSON, instead of as JSON number arrays, to avoid
text encode/parse overhead on the hot path. The JSON envelope describes the
layout; the receiver splits the block accordingly.

- `eval_batch` request: `i64` `xs_discrete_row_major` then `f64`
  `xs_continuous_row_major` (lengths from the JSON offsets or fixed domain
  widths). Response: the `f64` `values_row_major` block (length `nr_samples *
  len(components)`).
- `generate` response: `i64` discrete, `f64` continuous, `f64`
  weights (lengths from the JSON offsets or fixed domain widths).
- `feedback` request: the `f64` training-values block (`nr_values`
  in the JSON envelope).

Other batched methods (`pdf`, `transform_batch`, `materialize_batch`) currently
still pass their arrays as JSON numbers.

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
  "protocol": "gammaboard-jsonrpc-v4",
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

`eval_batch` evaluates a batch in ragged row-major form. When present, the
offset arrays have length `nr_samples + 1`; sample `i` uses
`row_major[offsets[i]..offsets[i + 1]]`. An offset array may be omitted only
when the corresponding width is fixed across the initialized domain. Workers
must support explicit offsets so domains with varying discrete depth or
continuous dimensionality remain representable.

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
  "protocol": "gammaboard-jsonrpc-v4",
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

`generate` receives `{ "max_samples": 8192 }`, a required positive integer.
The runtime caps this by queue `max_generation_size` and any remaining task
budget. The sampler returns between one and this many samples, possibly fewer
at a training boundary. It returns:

```json
{ "kind": "batch", "nr_samples": 2048, "training_remaining": 10000 }
```

The frame's binary payload contains `i64` discrete coordinates, `f64` continuous
coordinates, then `f64` positive finite weights. Offset metadata follows the same
fixed/ragged layout rules as other batched operations. Set `training_remaining`
to `null` for no feedback, or a positive count at least as large as this draw.

`{ "kind": "waiting" }` waits for outstanding feedback and
`{ "kind": "finished" }` ends generation. Neither carries a binary payload.

`feedback` receives `{ "nr_values": 2048 }` with a binary `f64` array: one already
weighted training scalar per sample of exactly one generated draw, in generation
and sample order. It returns `{ "ok": true }`. Model updates remain sampler-owned
and may combine multiple generation draws. Snapshots must retain private pending
training state. See [the complete sampler contract](sampling.md).

Protocol v4 replaces the optional remaining-task budget with a required positive
`max_samples` per-draw limit. Update custom workers and the SDK together.
Framing, binary layouts, feedback, and evaluator callbacks are unchanged.

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
  "protocol": "gammaboard-jsonrpc-v4",
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
  "protocol": "gammaboard-jsonrpc-v4",
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
points. `latent_batch` is the JSON form stored by GammaBoard; for current
samplers this is usually an `indexed_batch` payload containing discrete
signatures, per-sample discrete-map entries, continuous layouts/values, and
weights. Native Havana inference instead uses `havana_inference_indexed`
(RNG checkpoints plus a sample offset); its built-in materializer also accepts
legacy `havana_inference` single-seed payloads. See [sampling.md](sampling.md).

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

The optional `gammaboard-process` package implements this protocol for
homogeneous Python runtimes. Its installation, class contracts, logging helper,
and working examples are documented in
[../process_api/README.md](../process_api/README.md).

## Benchmark

Measure process API overhead with the optimized GammaBoard executable:

```bash
just benchmark protocol --python /path/to/venv/bin/python --output results/protocol
```

The command produces separate plots and a local HTML report automatically. It measures
the production Rust adapters and Python SDK for evaluator calls, sampler generation
and feedback, with feedback disabled/enabled. Paired callback timings separate
user computation from adapter overhead. See [benchmarking.md](benchmarking.md#process-protocol)
for measurement scope. Functional assertions remain separate in `tests/process_api.rs`.
