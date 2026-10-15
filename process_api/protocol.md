# Process wire contract (v6)

For SDK use and callback ownership, see [the process guide](README.md).
Requests are synchronous JSON-RPC 2.0 over stdin/stdout: one outstanding request
per process, matching response IDs, exactly one `result` or `error` object.
Logs go to stderr. Workers must reject an unsupported protocol string.

```text
Content-Length: <JSON byte length>\r\n
Binary-Length: <binary byte length>\r\n
\r\n
<UTF-8 JSON><binary>
```

`Binary-Length` is optional when zero. All binary numbers are little-endian.
Responses use `{ "jsonrpc":"2.0", "id":1, "result":{...} }`; errors use
`"error": {"code":-32000,"message":"...","data":{...}}` instead of `result`.

## Initialization

```json
{
  "protocol": "gammaboard-jsonrpc-v6",
  "role": "evaluator",
  "domain": {"rectangular":{"discrete_cardinalities":[],"continuous_dims":2}},
  "output": {"kind":"values","components":["value"]},
  "args": {}
}
```

Roles: `evaluator`, `sampler`, `batch_transform`, `materializer`. Only evaluators
receive `output`. Samplers additionally receive `evaluator_metadata` and `snapshot`
(null for fresh initialization). Return `{"ok":true}`; evaluators can add
`"metadata": {...}`. Domain layout is authoritative; branching `discrete` domains
may have varying coordinate widths.

## Array layout

Rows are sample-major. Discrete coordinates are `i64`, continuous coordinates,
weights, and values are `f64`. `xs_discrete_offsets` / `xs_continuous_offsets`
contain `nr_samples + 1` entries, start at zero, are nondecreasing, and end at the
corresponding flat array length. Offsets can be omitted only for dimensions whose
width is fixed by the initialized domain. Direct workers must support ragged input.
Coordinates must be finite and match the domain; sampler/transform/materializer
weights must be positive and finite. Responses must contain the exact requested
array lengths. Callback results cannot contain NaN or infinity.

| Method | JSON parameters | Binary request | Result JSON / binary |
| --- | --- | --- | --- |
| `eval_batch`, values output | `nr_samples`, `components`, optional offsets | Discrete, continuous | `{}` / sample-major values, `N × components` doubles |
| `eval_batch`, GammaLoop output | `nr_samples`, `require_training_values`, optional offsets | Discrete, continuous, `N` total weights | `{"gammaloop_state":...}` / `N` weighted training doubles when requested, otherwise empty |
| `generate` | Positive `max_samples` | None | `{"kind":"batch","nr_samples":N,"training_remaining":null-or-count,...offsets}` / discrete, continuous, `N` weights |
| `feedback` | `nr_values` | One draw's weighted doubles | `{"ok":true}` / none |

Generation can instead return `{"kind":"waiting"}` or `{"kind":"finished"}`
without binary data. Feedback order, training windows, and snapshot obligations
are defined in [the sampler contract](README.md#sampler-feedback-and-restoration).
GammaLoop state is the JSON representation of the current native accumulator;
[the observable example](examples/python_gammaloop_observable/src/demo_evaluator.py)
shows a complete producer. GammaBoard does not reweight that state.

## JSON-only methods

The remaining methods carry arrays in JSON. A concrete row-major batch contains
`xs_discrete_row_major`, `xs_continuous_row_major`, offsets, and `weights`.

| Method | Parameters | Result |
| --- | --- | --- |
| `transform_batch` | `nr_samples` and concrete batch fields | Transformed concrete batch fields |
| `materialize_batch` | `nr_samples`, `latent_batch` | Concrete batch fields |
| `pdf` | `nr_samples`, coordinate arrays/offsets | `{"values": [number-or-null,...]}` or `{"values":null}` |
| `discrete_pdf` | `subspaces: [{fixed_dims:[{dim:0,value:2}]}]` | Same values form as `pdf` |
| `snapshot` | `{}` | `{"snapshot": <non-null JSON>}` |
| `diagnostics` | `{}` | `{"diagnostics": {}}` by default |
| `global_pdf_norm` | `{}` | `{"norm":1.0}` by default; finite and positive |
| `persisted_output` | `{}` | `{"output":null}` by default |

`diagnostics` applies to evaluators and samplers; snapshot/PDF/output methods
apply to samplers. Implement default responses for unsupported optional features;
unknown-method errors are not capability negotiation.

`latent_batch` is `{nr_samples,payload}`. An `indexed_batch` payload holds
`discrete_signatures`, per-sample `discrete_map`, `continuous_layouts`,
`continuous_values`, and `weights`; `havana_inference` holds RNG checkpoints and
an offset. See [the payload types](../src/sampling/latent_batch.rs) when implementing
a materializer for those representations.

Workers may emit JSON-RPC `progress` notifications before their response, carrying
`{"activity":"phase description"}` for display. Notifications do not complete a request.
`@gblog\t<level>\t<message>` on stderr records a structured worker log.
EOF initiates shutdown; see [lifecycle](README.md#logging-and-shutdown).
