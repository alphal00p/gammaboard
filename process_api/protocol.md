# Process wire protocol v10

This is the complete transport contract for `gammaboard-jsonrpc-v10`. The
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
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":"gammaboard-jsonrpc-v10","role":"sampler","domain":{"rectangular":{"discrete_cardinalities":[],"continuous_dims":2}},"args":{},"evaluator_metadata":{},"initialization":{"kind":"fresh"}}}
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
| `protocol` | Exactly `gammaboard-jsonrpc-v10` |
| `role` | One of the four role strings above |
| `domain` | Structural domain; optional/null for descriptor-based evaluator discovery |
| `args` | User construction arguments, as a JSON object |

Additional parameters:

| Role | Parameters |
| --- | --- |
| Legacy evaluator | `output: {kind:"values", components:["value",...]}` or `{kind:"gammaloop"}` |
| Sampler | `evaluator_metadata`: JSON from the evaluator; `initialization`: tagged `fresh`, `restore`, or `from_stage` object |
| Materializer / transform | No additional initialization fields |

All roles return `{"ok":true}` without binary data. Evaluators can additionally
return `metadata`, which GammaBoard forwards to sampler initialization. An absent
metadata field means `{}`. Evaluator components are ordered, unique names; their
order fixes the component axis in returned value arrays.

Sampler workers may additionally return `"checkpoint_committed": true` to opt in
to the checkpoint notification RPC below. Omitted or false means GammaBoard does
not send that RPC. This optional extension does not change the v10 version string.

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

## Selected-output evaluator contract

New evaluators opt in during `initialize` with:

```json
{"ok":true,"evaluation_protocol":"gammaboard-evaluation-v1",
 "descriptor":{"domain":{"rectangular":{"discrete_cardinalities":[],"continuous_dims":1}},
               "components":["real","imag"],"scalar_projection":"abs",
               "observables":true,"details":true,"diagnostics":true},
 "evaluation_encodings":["gammaboard-evaluation-binary-v1","json"],"metadata":{}}
```

The descriptor declares available channels, not a fixed output mode. Components
are nonempty unique names. `scalar_projection` is optional (`real`, `imag`, `abs`,
`abs_sq`) and requires `real`/`imag`; the host expands synthetic `value` requests
and projects centrally. A declared real `value` component takes precedence.
Optional `observable_schema` is a cleared snapshot bundle; otherwise the binary
response establishes its schema lazily. Missing capability booleans default to false.

The run card needs only `command` and `args`. Optional `domain` and `descriptor`
are assertions checked against the worker. Run validation discovers and caches the
descriptor by starting the executable on the service host. Supply an explicit
`descriptor` when that host cannot execute the worker; runtime initialization still
checks it. Discovery performs normal worker initialization, so use read-only state
loading and explicit paths. Legacy cards with both `domain` and `output` remain
validatable without starting a worker.

Each `evaluate` request contains the usual `nr_samples` and coordinate offsets,
plus `outputs` and `encoding`. Its input binary is unchanged:

```json
{"nr_samples":2,"outputs":{"components":["imag","real"],"observables":false,
                             "details":true,"diagnostics":true},"encoding":"json"}
```

A JSON response is `{"samples":[{"values":[...],"details":{...},"diagnostics":{...},
"observables":{...}}, ...]}` with no binary block. Omit unrequested channels.
There must be exactly one finite value row per input, in original input order and
requested component order. Unsupported channels fail explicitly. Different calls
on the same initialized evaluator may select different components/channels.

`details` contains `integrand_value_re`, `integrand_value_im` and optional/null
`parameterization_jacobian`. Values already include any evaluator parameterization
Jacobian; details describe the raw integrand and do not trigger another multiplication.
Only GammaBoard applies outer sampling weights, stratum masking and reduction.
Observable bundles describe one original draw (`sample_count=1`), including empty
draws; correlated fills are summed before squaring the per-bin contribution.

`diagnostics` is the per-sample GammaLoopDiagnostics record below: `count_total=1`,
nonnegative counters and finite nonnegative timings. Missing diagnostics are
unavailable, not measured zero. The two optional canonical preparation timings
may individually be absent/null; an explicit zero means measured zero.

### Compact selected output

A binary response is `{"encoding":"gammaboard-evaluation-binary-v1"}`, optionally
with `observable_schema` on the first observable response. All numbers are little
endian, rows are concatenated without padding, and each row has:

1. `C` f64 values in requested component order.
2. If observables: the sparse histogram records from the compact observable format
   below, **without** its two leading real/imag f64 values. Histogram names are sorted.
3. If details: two f64 raw integrand components, then optional Jacobian.
4. If diagnostics: nine i64 counters in this order: `count_total`,
   `count_double_precision`, `count_quad_precision`, `count_arb_precision`,
   `count_nan`, `count_nan_or_unstable`, `count_loop_momenta_escalated`,
   `total_generated_events`, `total_accepted_events`; five f64 timings:
   `total_eval_time_ms`, `total_integrand_eval_time_ms`, `total_evaluator_eval_time_ms`,
   `total_parameterization_time_ms`, `total_event_processing_time_ms`; then optional
   `total_canonical_sampling_preparation_time_ms` and
   `total_canonical_physical_preparation_time_ms`.

Each optional f64 uses a u8 tag (`0` absent, `1` followed by f64 present).
No extra bytes are permitted. Schema is retained across changes in output selection;
empty batches need no schema. Binary requires stable histogram metadata. Select JSON
for changing layouts. Both encodings have identical semantics.

This is an evaluator-only negotiated extension; sampler/materializer/transform
remain on v10 unchanged. Without the extension, GammaBoard adapts `eval_batch`
results to the shared contract. Existing observable workers can serve projected
scalar/subset tasks, but cannot provide missing details or diagnostics. The Rust
GammaLoop wrapper also retains `eval_batch` for older hosts. New Python providers
implement `descriptor()` and `evaluate(...)`; old `eval(...)` providers still work.

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
| Evaluator / `eval_batch`, observable output | `nr_samples`, `components:["real","imag"]`, optional coordinate offsets | `D` i64 discrete, `X` f64 continuous | `{"observables":[bundle_0,...,bundle_N_minus_1]}` | `N*2` unweighted f64 real/imaginary values |
| Sampler / `generate` | Positive integer `max_samples` | Empty | See generation results below | For a batch: `D` i64 discrete, `X` f64 continuous, `N` f64 weights |
| Sampler / `feedback` | `nr_values`, `stratum_prefixes` (array of complete integer tuples or null) | `N` unmasked weighted f64 values; when prefixes are present, `N` requested u64 table indices followed by `N` actual u64 table indices | `{"ok":true}` | Empty |

Sampler feedback prefix tables are sorted in numeric lexicographic order, nonempty,
and prefix-free. Both index arrays have exactly `N` entries in range. The SDK exposes
one `Feedback` object: `.values` is masked by requested/actual index equality;
`.unmasked_values` preserves misses; `.strata` exposes both assignments. Without a
partition, `stratum_prefixes` is null and only the value block is present. Splitting
and recovery preserve the complete feedback object and original draw boundaries.

The `components` field on legacy `eval_batch` requests agrees with initialization.
Selected-output `evaluate` requests instead choose a subset/order per call.
Every evaluator returns exactly one unweighted result per input. Observable bundles
use the GammaLoop histogram snapshot schema, but each histogram has sample_count=1.
Sum all correlated fills within each bin before squaring; include empty draws.
The host validates these counts and moments and applies sampling weights,
classification, masking, feedback projection and per-stratum reduction centrally.
Evaluators never receive outer weights or stratification metadata and cannot
return aggregate statistics. See the [observable example](examples/python_observables/src/demo_evaluator.py)
and [typed output](python/src/gammaboard_process/evaluation.py).
Legacy initialization may specify `output.training_projection` for observable output:
`real` (default), `imag`, `abs`, or `abs_sq`; this is interpreted by the Rust host.

### Optional compact observable transport

An evaluator can advertise
`observable_encodings: ["gammaboard-observables-binary-v1"]` in its successful
initialize result. A supporting host opts in by adding
`observable_encoding: "gammaboard-observables-binary-v1"` to each observable
`eval_batch` request. Without that request field, the evaluator must retain the
JSON observable contract above. Initialization requests and the v10 protocol
identifier are unchanged; existing Python/older workers remain compatible.

A compact response has `observable_encoding` with the same value. Its first
successful nonempty response also contains `observable_schema`, a GammaLoop
`ObservableSnapshotBundle` with all sample/entry/mitigation counters and moments
zeroed. It contains the histogram and bin metadata; subsequent responses omit
it. Histogram names are traversed in ascending UTF-8 lexical order. The layout
is immutable for the process lifetime; use JSON mode for dynamic layouts.
The response omits the legacy `observables` array.

The binary block is sample-major, with no padding. Each original sample contains:

1. Two little-endian f64 values: real and imaginary, including the
   parameterization Jacobian, before outer sampling weights.
2. For each histogram in schema order: three little-endian u64 counters
   (`in_range_entry_count`, `nan_value_count`, `mitigated_pair_count`), then a
   little-endian u32 populated-bin count.
3. For each populated bin: u32 bin index, u64 `entry_count`, f64 `sum_weights`,
   f64 `sum_weights_squared`, u64 `mitigated_fill_count`, all little-endian.

Bin indices address the schema's `bins` vector followed by underflow and overflow.
Indices must be strictly increasing and in range. A bin is omitted only when all
its counters and moments are zero; zero-valued fills with nonzero counts remain
present. Every histogram implicitly has `sample_count=1`, including empty draws.
All floats must be finite. Moments still square the complete correlated sample
contribution, and the existing host validation, weighting, masking and reduction
rules apply unchanged. Counters must fit the host's integer range.

The host requires exactly `nr_samples` rows and rejects truncated/trailing data,
duplicate or out-of-range indices, changed/repeated schemas, and unnegotiated
encodings. An empty batch has an empty binary block; schema discovery can wait
until the first nonempty batch. Schema and binary streams reset when a worker is
restarted. No aggregate histogram statistics cross this interface.

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
| Sampler / `checkpoint_committed` (advertised at initialization) | `{"snapshot": <exact committed snapshot state>}` | `{"ok":true}` |
| Sampler / `persisted_output` | `{}` | `{"output":null}` by default |
| Evaluator or sampler / `diagnostics` | `{}` | `{"diagnostics":{}}` by default |

For unavailable PDF information, return null or null entries. Default optional
methods still have successful responses; an unknown-method error is not a default.
`persisted_output` is display/task output, distinct from recoverable private state.
`snapshot` must preserve all state needed to process pending feedback after restore.
`checkpoint_committed` is sent only after the database save succeeds. Its payload
is the opaque state returned by `snapshot`, not the enclosing GammaBoard checkpoint.
Notification errors leave that checkpoint committed and are retried after 30 seconds
while the runner is active. A newer save attempt supersedes an older pending notice;
after an uncertain commit, an old notice could incorrectly authorize cleanup.
Delivery is best effort and may repeat; implementations must be idempotent.
On restore, the supplied snapshot is authoritative even if no notification arrived.
Acknowledgement does not release artifacts referenced by other stages or backups.
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
