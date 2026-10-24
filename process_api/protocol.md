# Process wire protocol v9

This is the complete transport contract for `gammaboard-jsonrpc-v9`. The
[execution specification](../docs/execution-model.md) defines sampling, weighting,
feedback ordering, and recovery semantics. The [process guide](README.md) covers
SDK setup. The protocol has four roles: `evaluator`, `sampler`, `materializer`,
and `batch_transform`.

## Framing and RPC lifecycle

GammaBoard writes requests to a child process's stdin and reads responses from
stdout. Each process handles one synchronous request at a time. Every request has
`jsonrpc: "2.0"`, an `id`, a method name, and a `params` object. A response echoes
that `id` and contains exactly one of `result` or `error`:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":"gammaboard-jsonrpc-v9","role":"sampler","domain":{"rectangular":{"discrete_cardinalities":[],"continuous_dims":2}},"args":{},"evaluator_metadata":{},"initialization":{"kind":"fresh"}}}
```

```json
{"jsonrpc":"2.0","id":1,"result":{"ok":true}}
```

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"description","data":{}}}
```

Each message is framed as:

```text
Content-Length: <UTF-8 JSON byte length>\r\n
Binary-Length: <binary byte length>\r\n
\r\n
<JSON bytes><binary bytes>
```

`Binary-Length` is optional when zero. There is no newline delimiter after the
payload. Read exactly the declared byte lengths; a stream read can return a
partial frame. Binary arrays are little-endian and contiguous. JSON must contain
finite values. Binary floating-point values must also be finite where numerical
inputs/results are required. Stdout must not contain unframed logging.

The first method is `initialize`. Unsupported protocol strings and wrong roles
are errors. Requests for other methods before initialization are errors. Unknown
methods return errors; they are not optional-feature negotiation. A callback
error may be returned without exiting the worker; GammaBoard's batch/task failure
policy decides what to retry. A worker process exiting or truncating a frame is
an execution failure.

## Initialization

All roles receive:

| Parameter | Meaning |
| --- | --- |
| `protocol` | Exactly `gammaboard-jsonrpc-v9` |
| `role` | One of the four role strings above |
| `domain` | The structural domain described below |
| `args` | User construction arguments, as a JSON object |

Additional parameters:

| Role | Parameters |
| --- | --- |
| Evaluator | `output: {kind:"values", components:["value",...]}` or `{kind:"gammaloop"}` |
| Sampler | `evaluator_metadata`: JSON from the evaluator; `initialization`: tagged `fresh`, `restore`, or `from_stage` object |
| Materializer / transform | No additional initialization fields |

All roles return `{"ok":true}` without binary data. Evaluators can additionally
return `metadata`, which GammaBoard forwards to sampler initialization. An absent
metadata field means `{}`. Evaluator components are ordered, unique names; their
order fixes the component axis in returned value arrays.


Sampler initialization is exactly one of:

```json
{"kind":"fresh"}
{"kind":"restore","state":{"...":"private checkpoint"}}
{"kind":"from_stage","source_state":{"...":"trained model"},"source_args":{}}
```

`restore` preserves RNG state, counters and pending feedback. `from_stage` passes
an earlier sampler's private state and source arguments to the destination's
explicit handoff hook; destination `args` configure the new stage. The adapter
transfers the learned proposal into fresh counters/feedback state. An unsupported
handoff fails explicitly. The Python hooks are `from_snapshot` and `from_stage`.
Initialization requires the tagged form above; nullable `snapshot` fallbacks are unsupported.

For stratification the sampler/materializer receives the augmented domain; evaluator
and transform callbacks receive the integrand domain. Stratum axes form a left-bound
prefix on each path, with possibly different lengths. The partition maps each complete
prefix to one centre. `discrete_mass` queries fixing a complete prefix must return its
exact normalized selection probability for stratified inference. Other queries may
return unspecified entries. These probabilities agree with generated full weights;
they are neither observed frequencies nor Voronoi-region volumes. The frozen plan
is queried once and checkpointed. Independent frozen sampling is a documented
statistical assumption, not a capability flag.

## Domains and sample arrays

A rectangular domain is:

```json
{"rectangular":{"strata_dims":0,"discrete_cardinalities":[3,2],"continuous_dims":4}}
```

A branching domain is:

```json
{"discrete":{"axis_label":"channel","axis_role":"integrand","branches":[{"index":0,"domain":{"rectangular":{"discrete_cardinalities":[],"continuous_dims":2}}},{"index":1,"domain":{"rectangular":{"discrete_cardinalities":[],"continuous_dims":3}}}]}}
```

A branch's `index` contributes the next discrete coordinate. Labels are increasing,
unique, nonnegative integers; gaps are allowed. `axis_role` is `"stratum"` or
`"integrand"` (default). A rectangular node marks its first `strata_dims` axes as
strata (default zero). Along every path, strata must precede integrand axes.
Different paths may have different stratum-prefix lengths. A rectangular leaf
adds its discrete coordinates, each in `[0, cardinality)`, and fixes the number
of continuous coordinates. The example therefore has one discrete coordinate
per sample and either two or three continuous coordinates. Continuous coordinates
must be finite. The domain specifies structure; it does not impose continuous
numerical integration bounds.

For `N = nr_samples`:

- Discrete coordinates are flat sample-major `i64` values.
- Continuous coordinates are flat sample-major `f64` values.
- Weights and training feedback are `N` `f64` values.
- Ordinary evaluator values are sample-major `N × C` `f64` values, with components
  ordered as declared at initialization.

`xs_discrete_offsets` and `xs_continuous_offsets`, when present, are JSON arrays
of `N+1` nonnegative integer offsets. They start at zero, are nondecreasing, and
end at the corresponding flat coordinate length. Consecutive offsets delimit a
sample's row. An offsets array may be omitted only when that coordinate width is
fixed by the initialized domain; the length is then `N × fixed_width`. A zero-width
coordinate array is empty. Explicit fixed-width offsets must match that layout.

The Python SDK passes the full typed `Domain` to constructors, `from_snapshot`,
and `from_stage`. Fixed-width coordinate blocks use NumPy matrices; variable-width
blocks use tuples of 1D NumPy arrays. Every callback input array owns writable storage.
All roles use the same codec and domain validation.

## Binary methods

Here `D` is the total number of discrete coordinates in the message and `X` the
total number of continuous coordinates, determined by offsets or fixed widths.
There is no binary padding between blocks. The response must have exactly the
specified byte length; extra/truncated data is invalid.

| Role / method | Request JSON parameters | Request binary blocks, in order | Result JSON | Result binary blocks |
| --- | --- | --- | --- | --- |
| Evaluator / `eval_batch`, values output | `nr_samples`, `components`, optional coordinate offsets | `D` i64 discrete, `X` f64 continuous | `{}` | `N*C` unweighted f64 component values |
| Evaluator / `eval_batch`, GammaLoop output | `nr_samples`, `require_training_values` boolean, `stratification` (object or null), optional coordinate offsets | `D` i64 discrete, `X` f64 continuous, `N` f64 total weights | `{"gammaloop_state":...}` | First `N` weighted f64 training values if requested, then `N*2` unweighted real/imag f64 values if stratifying; omitted blocks have zero length |
| Sampler / `generate` | Positive integer `max_samples` | Empty | See generation results below | For a batch: `D` i64 discrete, `X` f64 continuous, `N` f64 weights |
| Sampler / `feedback` | `nr_values`, `stratum_prefixes` (array of complete integer tuples or null) | `N` unmasked weighted f64 values; when prefixes are present, `N` requested u64 table indices followed by `N` actual u64 table indices | `{"ok":true}` | Empty |

Sampler feedback prefix tables are sorted in numeric lexicographic order, nonempty,
and prefix-free. Both index arrays have exactly `N` entries in range. The SDK exposes
one `Feedback` object: `.values` is masked by requested/actual index equality;
`.unmasked_values` preserves misses; `.strata` exposes both assignments. Without a
partition, `stratum_prefixes` is null and only the value block is present. Splitting
and recovery preserve the complete feedback object and original draw boundaries.

The `components` field on ordinary evaluation requests agrees with initialization.
Ordinary callbacks return unweighted values; the Rust adapter handles accumulation
and training projection. GammaLoop callbacks apply total weights exactly once to
both native state and requested feedback. The native state is current
`GammaLoopAccumulatorState` JSON; use the
[observable example](examples/python_gammaloop_observable/src/demo_evaluator.py)
and [typed producer](python/src/gammaboard_process/gammaloop.py) for its structure.
GammaBoard does not apply another weight to that state. Native state may count
internal integration samples separately from the queued input sample count.

With stratification, the request object is
`{"partition":{"kind":"voronoi","domain":...,"centers":{"0":[...],"1,3":[...]},"scales":[...]},"labels":[...]}`.
Scales can be omitted (ones); labels identify the requested stratum for every
original draw. Dense labels index centres in numeric lexicographic prefix order.
The integrand coordinate arrays exclude the complete requested stratum prefix.
The callback must classify its raw real/imag values, mask contributions before
observable accumulation, and retain full input weights. A miss retains unmasked weighted training feedback and produces no histogram
fills, but counts as one sample in every
histogram and primary estimate. All events from one draw remain one statistical
unit. The response values block is sample-major `(real,imag)` and contains the
unmasked values, including on misses. Every value must be finite. No guessed or
recomputed probabilities are needed by this callback: the host alone applies its
frozen `p_k` to the primary integral estimator. The native state remains pooled.
The host requires every native estimate/histogram count to equal `nr_samples`
when stratifying. Independent integrations triggered by one queue point are
therefore not valid stratified observable producers.

Generation results are one of:

```json
{"kind":"batch","nr_samples":3,"training_remaining":6}
```

```json
{"kind":"waiting"}
```

```json
{"kind":"finished"}
```

A batch has `1 <= nr_samples <= max_samples`, positive finite weights, valid
coordinates, and optional coordinate offsets. `training_remaining` is null or
omitted for inference; otherwise it is an integer at least `nr_samples`, counting
the remaining training window before this draw. Waiting/finished results have no
binary payload. The runtime reassembles weighted feedback for each original draw;
feedback never combines draws or crosses draw boundaries. See
[generation and training windows](../docs/execution-model.md#generation-splitting-and-training-windows).

## JSON-only methods

These methods carry no binary request or response. A concrete batch has:

```text
xs_discrete_row_major: [i64, ...]
xs_continuous_row_major: [f64, ...]
xs_discrete_offsets: [integer, ...]     # optional for fixed-width discrete rows
xs_continuous_offsets: [integer, ...]   # optional for fixed-width continuous rows
weights: [f64, ...]
```

The same row-count, offsets, and domain rules apply. Materializer/transform output
weights are positive and finite. Transforms return replacement total weights,
including the input weight and any new Jacobian factors.

| Role / method | Parameters | Result |
| --- | --- | --- |
| Transform / `transform_batch` | `nr_samples` and all concrete batch fields | Concrete batch fields, preserving sample count/order |
| Materializer / `materialize_batch` | `nr_samples`, `latent_batch` | Concrete batch fields, preserving sample count/order |
| Sampler / `pdf` | `nr_samples`, coordinate row-major arrays, optional coordinate offsets; no weights | `{"values":[number-or-null,...]}` with one entry per point, or `{"values":null}` |
| Sampler / `discrete_mass` | `subspaces:[{"fixed_dims":[{"dim":0,"value":2},...]} , ...]` | The same values form, one entry per subspace |
| Sampler / `snapshot` | `{}` | `{"snapshot": <non-null JSON>}` |
| Sampler / `persisted_output` | `{}` | `{"output":null}` by default |
| Evaluator or sampler / `diagnostics` | `{}` | `{"diagnostics":{}}` by default |

For unavailable PDF information, return null or null entries. Default optional
methods still have successful responses; an unknown-method error is not a default.
`persisted_output` is display/task output, distinct from recoverable private state.
`snapshot` must preserve all state needed to process pending feedback after restore.
Materializers and transforms have no snapshot method.

`latent_batch` is `{nr_samples, payload}`; its count agrees with the outer count.
The payload is one of the current
[latent representations](../docs/execution-model.md#samples-domains-and-queue-payloads).
For example:

```json
{"nr_samples":2,"payload":{"kind":"indexed_batch","discrete_signatures":[[0]],"discrete_map":[0,0],"continuous_layouts":[2,2],"continuous_values":[0.1,0.2,0.3,0.4],"weights":[1.0,1.0]}}
```

An external sampler's binary coordinate output is encoded into this indexed form
by GammaBoard. Arbitrary new latent payload variants are not a process extension
point. `havana_inference` RNG state is internal serialized state tied to its saved
frozen grid and matching GammaBoard version.

## Notifications, logs, and shutdown

A worker may emit this framed notification before its response:

```json
{"jsonrpc":"2.0","method":"progress","params":{"activity":"phase description"}}
```

It has no response `id` and does not complete the outstanding request. Send logs
to stderr; `@gblog\t<level>\t<message>` records a structured log. The Python SDK
provides `gammaboard_process.log` and redirects `print` safely.

EOF on stdin initiates shutdown. GammaBoard waits `shutdown_grace_seconds`
(default 30), then sends SIGTERM to the process group; after another five seconds
it uses SIGKILL if needed. Do not rely on an extra shutdown RPC. Callback execution
and feedback can repeat after recovery; the
[replay contract](../docs/execution-model.md#tasks-checkpoints-and-failures) applies
to every process role.
