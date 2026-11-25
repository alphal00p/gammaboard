# GammaLoop observable transport diagnosis

This diagnostic complements the real-instance A/B harness. It isolates the
production evaluator adapters without a database, using the same saved state and
the frozen process-worker executable from an A/B run. Native and process paths
evaluate identical fixed Halton points on the same CPU; each evaluator is loaded
once and warmed before timing. The selected state must have a 3D continuous
domain and nonempty observables (the gg → HHH showcase meets these requirements).

```bash
cargo build --release --locked --example gammaloop_transport
taskset -c CPU target/release/examples/gammaloop_transport \
  --args /absolute/path/ab-results/inputs/args.json \
  --worker /absolute/path/ab-results/inputs/gammaloop_process \
  --batch-sizes 128,512,1024 --samples 8192 \
  --output /absolute/path/profile.json
```

The parent and process child inherit the same affinity. The output's `production`
measurements use the actual adapters and include result destruction. Four modes
separate native/process execution and values/observable output. Values-only mode
still uses the same native per-sample path, including creation of observable
bundles; those bundles are discarded rather than transported and reduced.
Histogram equality and numerical moment equality are checked automatically.

`decomposition_seconds` times a reconstruction of the original, unoptimized JSON observable pipeline:
input packing/decoding, native evaluation, JSON-tree construction, the response
envelope copy, byte encoding, host parsing, both host tree clones, conversion to
typed bundles, destruction, and the real host reducer. Binary values use the
same little-endian representation as the process API. This reconstruction uses
one allocator in one process and does **not** transfer bytes through pipes. Its
component timings are historical diagnostic estimates, not an exact additive profile of
the separate-process adapter. Allocator reuse, cache state, scheduling and pipe
framing explain why its sum need not equal the production measurements.
The production adapter measurements use the active implementation and negotiate
compact transport when the selected worker advertises it; the legacy JSON
reconstruction remains available to explain the original cost.

`typed_json_alternative` separately measures direct serialization/deserialization
of typed envelopes, including destruction. It preserves the current JSON schema
and checks a typed round trip. It is an isolated experiment, not an implemented
replacement or a measured end-to-end speedup. No production protocol or adapter
behavior is changed by this diagnostic.

`frame` records the actual JSON byte count and per-sample histogram/bin counts.
Initialization is reported separately and excluded from steady-state timings.
Raw per-call durations are retained; use enough samples and repeat measurements
before interpreting small differences on a shared host.
