# Process Runtime

GammaBoard can run evaluators and samplers as ordinary child processes. A process can be Python, Rust, C++, or anything else that can read stdin and write stdout.

## Contract

The extension protocol is `gammaboard-jsonrpc-v1`.

- Transport: JSON-RPC 2.0 messages framed with `Content-Length` headers.
- Direction: GammaBoard sends requests on process stdin; the process writes responses on stdout.
- Logging: stderr is for logs. GammaBoard also tolerates limited accidental line-oriented stdout before a protocol frame and forwards it to logs, but stdout should be treated as reserved for framed responses.
- Concurrency: requests are synchronous. GammaBoard sends one request at a time per process and waits for the matching response id before sending the next.
- Batching: evaluator `eval_batch`, sampler `produce_latent_batch`, sampler `ingest_training_values`, and sampler `pdf` are batched.
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
{ "ok": true }
```

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
  "snapshot": null
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

`snapshot` returns any JSON-safe state needed to restore the sampler:

```json
{ "snapshot": { "grid": "...", "seed": 42 } }
```

`get_diagnostics` returns optional JSON-safe diagnostics:

```json
{ "diagnostics": { "training_rate": 0.01 } }
```

## Config Shape

The process command is explicit in run config. GammaBoard does not append worker scripts or assume Python.

```toml
kind = "process_evaluator"
command = ["python", "-u", "runtimes/my_runtime/evaluator_worker.py"]
domain = { continuous = { dims = 2 } }
components = ["value"]
args = { module = "demo_integrand", class = "SinIntegrand" }
```

The protocol uses `domain` as the authoritative coordinate layout. Homogeneous wrappers may derive fixed dimensions from it internally, but run config should not define separate shape hints.

Domain variants are snake_case. `rectangular` is the compact form for fixed-cardinality discrete grids:

```toml
domain = { rectangular = { discrete_cardinalities = [2, 3], continuous_dims = 2 } }
```

Relative command entries that look like paths are resolved below the resources root. Absolute paths are used as-is.
Nix, Apptainer, virtualenvs, and system packages are all just ways to make this command available.

## Python Wrapper Pattern

The Python examples include worker scripts that implement this protocol and then load a normal Python class from `args.module` and `args.class`. This is only a convenience layer.

- `process_api/examples/python_scalar_sin/evaluator_worker.py`
- `process_api/examples/python_sampler_symbolica_havana/sampler_worker.py`

Evaluator classes implement `eval(xs_discrete, xs_continuous)`.

Sampler classes implement `sample_plan`, `produce_latent_batch`, `training_samples_remaining`, `ingest_training_values`, `snapshot`, and optional `pdf` / `get_diagnostics`.

Optional `from_config(...)` and `from_snapshot(...)` constructors receive homogeneous dimensions derived from `domain` plus the TOML `args` without the wrapper-only `module` and `class` fields.

## Benchmark

Run the ignored protocol benchmark with:

```bash
cargo test -q process_evaluator_eval_batch_protocol_benchmark -- --ignored --nocapture
```

The benchmark uses a tiny Python echo evaluator and measures real `eval_batch` framing overhead for small, medium, and large batches.
