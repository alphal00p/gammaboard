# Process-API overhead: corrected measurement and optimization

The native-generation baseline and final adapters were compared at **16, 256,
4,096, 32,768 and 131,072 samples**, both with and without training feedback.
At the default 32,768 samples, sampler overhead fell **35–38%** and evaluator
overhead fell **60%**. A separate, limited end-to-end comparison measured **6.5%
and 8.4%** higher accepted throughput, respectively without and with feedback.

[Before/after plot (PNG)](comparison.png) · [SVG](comparison.svg) ·
[Plot-only PDF](../consolidated/benchmark-plots.pdf) ·
[Presentation Markdown](../consolidated/benchmark-plots.md)

![Process adapter overhead before and after](comparison.png)

## What the benchmark correction explains

The earlier diagnosis was incorrect: **`Generation::into_batch()` only unwraps a
flat `LatentBatchSpec`**. It does not perform the per-sample expansion in
`LatentBatchPayload::into_batch()`. That expanding method was never called by this
benchmark. No measured improvement can be credited to removing that expansion.

The benchmark now retains the native generation, including its training-window
metadata, and explicitly destroys results inside the timer in both roles. The old
code also destroyed its unwrapped result inside the timer. Each Rust wall time
has its corresponding Python callback time subtracted; three warmup calls and
process startup are excluded. Measured calls increase from 4–64 to 32–128 per
case, reducing sensitivity to individual scheduling stalls. This changes the
measurement coverage, not production behavior.

The initial three-run old-wrapper/native comparison at 32,768 samples gave median
sampler overhead of **4.293 → 4.446 ms** without feedback and **4.603 → 4.616 ms**
with feedback: no speedup. The final confirmation also varied on the unchanged
evaluator path. This supports treating wrapper differences as measurement noise,
not a removed per-sample cost. Raw old-wrapper measurements remain available.

The evaluator benchmark already follows production: it calls `eval_batch` on a
materialized `Batch`, receives values through the real Python SDK and accumulates
in Rust. It does not add a benchmark-only conversion. Generic evaluators return
values for accumulation in both modes; feedback-on additionally keeps weighted
training values. Sampler feedback-on plots sum generation and feedback adapter
costs at the same batch size; they are not one multiplexed RPC.

## Before and after

These are arithmetic means of paired `wall − callback` times pooled across five
complete runs, alternating stage order. All calls include result cleanup. Parent
and child use the same two physical CPUs for every stage, on a shared host.
Numbers include validation, packing, IPC, decoding, Rust accumulation and
scheduling; they are **adapter overhead, not pure wire latency**.

| 32,768 samples | Native baseline | Final | Reduction |
| --- | ---: | ---: | ---: |
| Sampler, no feedback | 5.395 ms | 3.535 ms | 34.5% |
| Sampler, with feedback | 5.999 ms | 3.745 ms | 37.6% |
| Evaluator, no feedback | 7.893 ms | 3.153 ms | 60.1% |
| Evaluator, with feedback | 8.266 ms | 3.295 ms | 60.1% |

| Samples | Sampler, no feedback (µs) | Sampler, feedback (µs) | Evaluator, no feedback (µs) | Evaluator, feedback (µs) |
| ---: | ---: | ---: | ---: | ---: |
| 16 | 40.06 → 41.68 | 66.52 → 68.58 | 41.84 → 40.46 | 42.26 → 40.16 |
| 256 | 53.80 → 50.90 | 83.82 → 78.79 | 78.41 → 64.49 | 78.73 → 66.12 |
| 4,096 | 655.64 → 446.37 | 728.08 → 507.02 | 880.65 → 401.68 | 932.23 → 420.59 |
| 131,072 | 24,374.75 → 14,591.74 | 25,221.03 → 15,435.27 | 32,812.54 → 13,674.34 | 35,112.25 → 13,534.46 |

Tiny calls do not benefit consistently: 16-sample sampler cycles were about
1.6–2.1 µs slower. At 131,072 samples, reductions are 39–40% for the sampler and
58–61% for the evaluator. Shared-host variability limits precision; these results
are not universal latency guarantees.

## Individually tested changes

Each step was compared against its immediate predecessor over three alternating
runs, using all five batch sizes and both feedback modes. The table shows the
**median of run means** at 32,768 samples, in milliseconds. These sequential
experiments are separate from the final five-run means above; their differences
must not be added together.

| Step retained | Affected role | Without feedback | With feedback |
| --- | --- | ---: | ---: |
| Skip unused fixed-width offsets in Rust and Python | Evaluator | 6.453 → 4.112 | 6.886 → 4.214 |
| Pack coordinates straight into the binary frame | Evaluator | 5.669 → 2.558 | 5.999 → 2.674 |
| Own decoded sampler arrays directly | Sampler | 4.528 → 3.352 | 4.633 → 3.054 |
| Join Python array buffers directly | Sampler | 3.496 → 2.684 | 3.719 → 2.847 |

Skipping offsets eliminated allocations for fields that are not transmitted.
Direct evaluator packing removed intermediate coordinate vectors and another
copy. Every paired default-size evaluator trial improved for these steps. Flat
sampler construction reuses decoded continuous coordinates and weights; discrete
signature deduplication remains shared with the generic builder. Every paired
default-size sampler trial improved for that step.

For the Python step, the unchanged evaluator control improved too, so the full
RPC difference cannot be attributed to packing alone. Profiling justified testing
it: the old encoder consumed about **0.283 ms per 32k response** in the profiled
RPC run. An independent loop over identical arrays, including output allocation
and cleanup, measured median encoder cost **1.602 → 0.049 ms** at 32k and
**7.817 → 0.273 ms** at 131k. The allocation pattern differs from the live RPC
profile; these helper timings establish a copying reduction, not an additive RPC
saving. At 16 and 256 samples, the helper was about 0.55 and 0.30 µs slower.
The simpler one-frame construction and repeatable large-array benefit justify
keeping it without adding a special small-batch branch.

Receive-buffer copies remain unchanged. The profiled evaluator input decoder was
about **0.132 ms per 32k call**, much smaller than the full adapter cost. Retaining
those copies preserves independent, writable callback arrays without introducing
buffer lifetime complexity.

The public RPC API and wire bytes are unchanged. Variable-width inputs retain
offsets and the generic builder. Explicit offsets still pass through validation;
budget, count, training-window, finite positive weight, truncated-frame and
trailing-byte checks remain. No concurrency, shared memory or tuning knob was
added. The frontier measurement reader now also accepts real process evaluators
without mock-only sleep diagnostics, leaving those counters unknown.

## Targeted end-to-end confirmation

The protocol gains warranted one limited comparison: both sampler and evaluator
use their real process adapters, with one zero-delay evaluator, 32,768-sample
transport batches, 1,048,576-sample generations and one sampler I/O thread. Each
stage/repeat used a fresh private database, fixed disjoint allocations of two
sampler, two evaluator and four database cores, and the same queue settings.
Three repeats alternated stage and feedback order, with eight-second measurement
windows after warmup. CPU allocations were not exclusive to this benchmark.

| Accepted throughput | Native baseline | Final | Change |
| --- | ---: | ---: | ---: |
| Materialized, no feedback | 1.482 M/s | 1.579 M/s | +6.5% |
| Training, feedback | 1.487 M/s | 1.611 M/s | +8.4% |

All twelve windows were valid and adequately sampled; every paired direction was
positive, though the training ranges overlap. All six private databases shut down
cleanly. These short tests support a modest gain for this configuration. They do
**not** establish a new frontier ceiling or imply that protocol reductions become
equal end-to-end throughput gains. Historical frontier and isolated I/O plots were
not rerun; only the protocol slide in the four-slide presentation is updated.

## Validation and data

Validation passed: 344 Rust library tests (one unrelated test ignored), eight CLI
tests, the explicitly enabled process-API round-trip test, 27 Python SDK/MadGraph
tests (one skipped), 39 benchmark harness tests, Rust formatting, and Clippy over
all targets/features with warnings denied. Regression coverage checks byte layout,
ragged fallback and malformed frames, fixed-offset validation, strided/big-endian
Python output, and callback array ownership/writability across calls.

- [Manifest and snapshot hashes](manifest.json), [native-baseline benchmark patch](baseline-measurement.patch).
- [All final before/after aggregates](comparison.csv), [individual step trials](individual-trials.csv), [raw final paired trials](paired-trials.json).
- [Native baseline calls](baseline-measurements.json), [final calls](measurements.json), [old-wrapper calls](legacy-measurements.json).
- [Python profile](python-profile.txt), [isolated encoder trials](python-encoding-micro.json).
- [End-to-end summary](end-to-end/summary.json), [all twelve results](end-to-end/results.json), [configuration](end-to-end/manifest.json), [raw windows, run cards and cleanup records](end-to-end/raw-windows.json.gz).

The full local experiment, including immutable binaries/SDKs, per-stage source
patches and logs, is at
`/common/dev/cedric/setup-logs/protocol-overhead-20261001/`. Interrupted setup/debug
attempts are excluded from every reported comparison. To run the current protocol
suite on another machine, from the repository root:

```bash
python -m benchmarks protocol --batch-sizes 16 256 4096 32768 131072 --output results/protocol
```

Use an optimized current binary, as described in [benchmarking](../../../benchmarking.md).
