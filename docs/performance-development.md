# Performance findings

Use [benchmarking](benchmarking.md) for commands, settings and validity rules.
This page records current measurements and retained mechanisms; historical
experiments are not current deployment defaults.

## Current capability study — 2026-09-29

The sampler-generation implementation was measured with the default sparse
frontier: **87 configurations**, three data paths, four delays and up to 512
evaluator processes. Initial coverage, missing-point follow-up and targeted repeats
took 60.1 minutes including deployment and cleanup. Of 92 attempts, 89 were valid;
all planned configurations have usable evidence.

The shared host was an AMD EPYC 9754. One core served the sampler, fifteen the
database/server, and evaluators shared 240 physical cores. The build used
`dev-optim` (optimization level 2). Delayed evaluators sleep; these are
coordination/transport measurements rather than CPU-bound scaling results.

| Mode | Zero-delay peak samples/s | Evaluators at peak | Samples/s at 512 | Fewer-worker candidate within 5% |
| --- | ---: | ---: | ---: | --- |
| RNG inference | 30.56M | 64 | 28.65M | 16 workers, 524,288 samples/batch |
| Materialized | 2.62M | 256 | 2.21M | 16 workers, 131,072 samples/batch |
| Training | 2.56M | 128 | 1.89M | 64 workers, 131,072 samples/batch |

![Sparse throughput frontier](benchmarks/2026-09-29/frontier.svg)

At the zero-delay peaks, RNG sampler compute was 95.5% busy. Materialized/training
sampler I/O was 96.0%/95.6% busy, with sampler compute around 12–13%. Compute and
I/O overlap; these are occupied wall times, not CPU usage.

The 5-µs RNG curve used smaller 65,536-sample evaluator batches and peaked at
19.76M/s. Zero delay is not a universal ceiling for other batch choices.
At 200 µs, 256 evaluators delivered roughly 1.26–1.28M/s in all three modes,
close to the ideal 1.28M/s. Higher counts were less reliable:

| Repeated configuration | First samples/s | Repeat samples/s |
| --- | ---: | ---: |
| RNG, 5 ms, 512 evaluators | 67,175 | 101,298 |
| Training, 200 µs, 512 evaluators | 585,457 | 1,762,172 |

Both observations contribute to each plotted median. In the slow training
interval, roughly 500 batches remained locally queued while insertion was slow;
requested and actual mock sleep durations agreed. The fresh-deployment repeat
was faster. Publication/database contention is a useful next investigation,
but this does not isolate database history, CPU placement or shared-host load
as the cause. A short high-count point is not a stable deployment limit.

### Measurement corrections

One 512-evaluator training point exceeded the old 120-second warmup while making
progress. Warmup now allows five minutes within the suite budget. Two
single-evaluator training intervals missed feedback because a full generation
outlasted the old 12-second observation window. Training measurements now cover
two nominal generation cycles, independently of evaluator batches.

All three affected configurations passed their retries. Failures remain in the
evidence. The default suite now has a 60-minute limit and finishes early when
complete. Consolidation subsequently aligned accepted progress and feedback
with the same telemetry endpoints. Replaying all 89 valid intervals preserved
their rates and confirmed feedback inside every training interval.

## Process-API overhead

The production Rust/Python adapters completed 72 cases across nine batch sizes
from 16 to 1,048,576 samples in 74.7 seconds, excluding compilation. Inputs have
six continuous coordinates and one output component. Solid curves have minimal
callback work; dashed curves add 64 NumPy sine passes.

![Process API overhead](benchmarks/2026-09-29/process-overhead.svg)

These are paired adapter wall times minus callback wall times, including packing,
validation, IPC, native conversion/accumulation and scheduling. Startup, input
construction, the database and worker fleet are excluded.

At 65,536 samples per call with minimal callback work:

| Operation | Overhead per sample |
| --- | ---: |
| Sampler generation | 0.227 µs |
| Sampler feedback | 0.009 µs |
| Evaluator, feedback off | 0.347 µs |
| Evaluator, feedback on | 0.375 µs |

Batching amortizes fixed per-call cost; conversion and transfer leave a per-sample
cost. Million-sample calls showed an upturn in this run. Callback workloads also
produced different residuals, so these are not universal constants. The largest
batch has only four measured repetitions per case.

Process correctness checks passed, including weighted feedback and worker reuse.
[Portable measurements and provenance](benchmarks/2026-09-29/summary.json) include
all frontier attempts, configuration medians and process summaries. Full raw
intervals, paired timings and executable inputs remain in the development study
artifacts; the guide describes reproduction.

## Improvements already retained

These came from earlier, differently configured experiments. Their speedups are
not matched comparisons against the current frontier.

- **Shorter transaction lock ownership.** Queue-counter updates are deferred to
  commit, so large input COPY operations do not hold the shared counter lock
  throughout transfer. Metadata and payload visibility remain atomic.
- **Less payload copying.** Borrowed arrays serialize directly into the COPY
  buffer before connection acquisition. New PostgreSQL input writes use LZ4 where
  supported. Earlier eight-evaluator controls rose from about 0.87M/s to 2.29M/s
  at 65,536 samples/batch across the retained changes; serialization preparation
  alone did not establish a separate throughput gain.
- **Flat sampler output and reusable Havana samples.** A shared indexed builder
  removed temporary per-sample points. A six-dimensional 131,072-sample draw fell
  from 393,240 allocations to six and from 43.8 to 9.4 MB of allocation traffic.
  A historical matched Havana comparison rose from 0.39M/s to 2.12M/s; allocation
  and freeing had dominated the earlier profile.
- **One generation lifecycle.** Samplers own draws and training barriers.
  Splitting, ordered whole-draw feedback and recovery share one path. Refill depth
  is a soft fixed threshold. External samplers use SDK 0.2.0/protocol v3; see
  [sampling](sampling.md).
- **Bounded overlap and readiness.** Evaluator prefetch/submit stay bounded.
  Completed handles awaiting collection do not count as busy. Fleet readiness is
  separate from completed-work warmup; cleanup uses shared deadlines.

Keep claim tokens, retained results during database retries, task-scoped
consumption and checkpointed generation/feedback state: they protect correctness.
Historical CPU frontiers, constant-one feedback and process echo tests are
superseded as current capability evidence. Use their saved harnesses only for
older-revision investigations.

## Raising sampler I/O throughput

These are proposed experiments, not established speedups. Start with materialized
and training zero-delay cases at a modest fleet and at 512 workers. Hold binary,
batch, draw size, telemetry and placement fixed while changing one resource or
setting. Repeat against both fresh and aged databases.

| Change to scale | Potential benefit | Evidence needed |
| --- | --- | --- |
| Database CPU, memory/cache and storage bandwidth | Faster insertion, fetching, compression and maintenance | PostgreSQL CPU/waits, buffers, WAL/disk traffic, operation latency and accepted MiB/s |
| Insert lanes and connection capacity together | More publication overlap when the database has headroom | Compare 2/4/8 inserts; track pool waits and completion-collection starvation |
| Evaluator batches and insert bundles | More samples per claim/transaction | Nearby sizes at equal sample pressure; watch latency, memory and lock duration |
| Bounded parallel encoding | Relieve a saturated I/O thread during synchronous serialization | Profile CPU and separate serialization from database waits |
| Independent run/database partitions | Higher aggregate throughput past a shared database limit | Independent runs; a single adaptive sampler still requires ordered feedback |

The frontier uses two inserts; ordinary run defaults remain eight inserts.
Role pools are currently capped at two connections, shared by inserts, completion
fetches and persistence. A larger sampler pool requires a sampler-specific cap
and matching connection-admission accounting; setting a larger config value today
is clamped. Keep evaluator pools bounded when testing sampler capacity.
Public task tuning changes batch sizing, not insert concurrency. More encoding
threads also need more sampler CPU allocation to provide actual parallelism.

Separating large immutable payloads from PostgreSQL coordination is a larger
architectural option after bandwidth is established as the limit. Multiple
samplers for one adaptive run require a partitioning/training model, not simply
more processes.

Other focused follow-ups remain separate: GammaLoop entry-reset cost, finite
training windows/MadNIS behavior, and dependency numerical stability. An earlier
Havana fixture exposed tiny negative rounded variance for identical values of
1/6; varying benchmark feedback avoids that fixture but is not its numerical fix.
