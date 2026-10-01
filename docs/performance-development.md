# Performance findings

Use [benchmarking](benchmarking.md) for commands, settings and validity rules.
This page records current measurements and retained mechanisms; historical
experiments are not current deployment defaults.

## Sampler CPU and I/O-thread scaling — 2026-09-30

**Extra sampler cores help when the I/O thread count also increases.** Four
allowed physical cores with three background I/O threads gave the best result
that completed both workloads cleanly: **5.69M/s materialized and 6.07M/s
training**, respectively 21% and 40% above the repeated one-core/one-thread
baseline. Giving the existing one-thread runtime a second core did not establish
an improvement. These are deployment candidates; application defaults are unchanged.

| Sampler cores | I/O threads | Materialized samples/s | Training samples/s | Successful trials, materialized / training |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 4.69M | 4.33M | 4 / 4 |
| 1 | 3 | 4.19M | 4.59M | 1 / 2 |
| 2 | 1 | 4.79M | 4.22M | 2 / 2 |
| 2 | 3 | 4.62M | 5.46M | 1 / 2 |
| 4 | 2 | 5.42M | 5.51M | 2 / 2 |
| 4 | 3 | 5.69M | 6.07M | 2 / 2 |
| 8 | 4 | 5.42M | Cleanup failed twice | 2 / 0 |

Values are medians of successful trials. The two materialized CPU-allowance
checks with three I/O threads have only one trial each. Compare configurations
within a column: materialized used 16 active evaluators, training used 64.

![Sampler CPU and I/O-thread comparisons](benchmarks/2026-09-30/sampler-cores.svg)

The trials used one sampler/model and one private PostgreSQL database, with 64
registered evaluators. Fixed settings were six sampler connections, four concurrent
inserts, five batches per insert, 131,072 samples per evaluator batch, 4,194,304
samples per generation, and 4GB PostgreSQL shared buffers. The database/server had
15 allowed physical cores, and evaluators each had a separate core and one I/O
thread. All core sets were on the same CPU package and remained disjoint. Affinity
limits placement; it does not reserve cores against other host users.

Each interval followed the frontier warmup contract and requested 25 seconds of
measurement. The opening baseline was repeated after the main sweep. A further
fixed-three-thread comparison varied only the sampler's CPU allowance, with
reversed order for its repeated training trials. There were **28 successful
intervals and two excluded cleanup failures**, taking 24 minutes across the
private deployments, including warmup and shutdown. This remains a short,
shared-host, six-dimensional `f(x) = x[0]` transport study with zero evaluator
delay, not a sustained HPC-network or real-optimizer benchmark.

Serialization executes synchronously inside spawned I/O tasks. Multiple runtime
threads allow independent bundles to encode while other tasks handle database
traffic. In the fixed-three-thread training comparison, mean serialization wall
time per five-batch bundle fell from approximately **128 ms on one core**, to
**73 ms on two**, to **46 ms on four**. Training result-fetch time was about
4.3 ms with four cores/three threads versus 10.0 ms for the repeated baseline.
These observations support reduced scheduling delays and better overlap; they
do not imply parallel mutation of the sampler model. Average sampler CPU use at
four cores/three threads was only 0.92 cores materialized and 1.11 training:
short parallel bursts matter even when average CPU use is modest.

Both attempts at eight cores/four threads completed their first materialized run,
then failed cleanup after the training interval. The measured training rates,
6.35M/s and 6.07M/s, are retained as excluded points. PostgreSQL logged 30-second
statement timeouts in completed-batch deletion and subsequent run removal, both
while cascading into `batch_inputs`. Frequent WAL checkpoints and buffer/write
waits were also observed. The precise cause of the slow deletion is not isolated;
the evidence does not establish that four threads themselves cause it. The retry
kept all database limits unchanged. Both private deployments shut down cleanly,
and the failed databases/logs were retained for investigation.

For a high-throughput sampler node, **four cores with three sampler I/O threads**
is the strongest clean candidate from this study. These experiments varied the process-wide
`TOKIO_WORKER_THREADS`; the dedicated sampler pool added afterward exposes
`[sampler_aggregator_runner_params] io_threads = 3` instead and leaves the control
runtime separate. The numbers above describe the original experiment, not a
remeasurement of the new pool. Two cores/three threads recovered much of the
training gain. Additional cores or threads did not yield proportional scaling.
Payload cleanup and PostgreSQL checkpoint pressure need investigation before
raising throughput further. No production runtime code or configuration defaults
were changed for this experiment.

The [portable evidence](benchmarks/2026-09-30/sampler-cores.json) includes all
successful trials, excluded measurements, thread-affinity verification data and
binary/harness provenance. Reproduction scripts and raw logs are retained under
`/common/dev/cedric/setup-logs/sampler-cores-20260930/`.

## Sampler I/O tuning — 2026-09-30

**Six sampler connections and four concurrent inserts** improved the targeted
materialized/training comparisons and are now the run defaults. Evaluators remain
capped at two connections. Sampler admission reserves its four extra connections
without charging that overhead to every evaluator. Shutdown now joins an existing
cleanup DELETE before checkpointing or removing the run.

Matched zero-delay comparisons used `f(x) = x[0]`, six-dimensional inputs,
131,072-sample evaluator batches, 4,194,304-sample generations, five batches per
insert, one sampler core and fifteen PostgreSQL/server cores. Each confirmation
used the frontier warmup contract followed by at least 20 seconds of measurement.
The smaller studies registered 64 evaluators; the large-fleet study registered 512.
All trials used fresh runs within their study's ageing private database.

| Workload | Active evaluators | PostgreSQL shared buffers | 2 connections / 2 inserts | 6 connections / 4 inserts | Change |
| --- | ---: | ---: | ---: | ---: | ---: |
| Materialized | 16 | 256MB | 2.70M/s | 3.77M/s | +39% |
| Training | 64 | 256MB | 2.48M/s | 3.27M/s | +32% |
| Materialized | 16 | 4GB | 3.54M/s | 4.74M/s | +34% |
| Training | 64 | 4GB | 3.44M/s | 3.96M/s | +15% |
| Materialized | 512 | 4GB | 2.55M/s | 3.35M/s | +32% |
| Training | 512 | 4GB | 1.99M/s | 3.84M/s | +93% |

The 256MB confirmation rows are medians of two trials per configuration, with
reversed order in the second block. The smaller 4GB studies have two baseline
trials and one candidate; the 512-worker comparisons have one trial each. These
shared-host observations establish useful settings, not precise universal gains.
This is a targeted follow-up, not a replacement for the full sparse curves below.

![Sampler I/O comparisons](benchmarks/2026-09-30/sampler-io.svg)

The mechanism is bounded publication overlap with room left for result collection.
In one matched materialized pair, the reported mean completion-fetch time fell
from roughly 183 ms to 3.6 ms. Individual inserts can take longer while aggregate
throughput rises because four can run concurrently. Four connections with four
inserts were less effective; eight connections or eight inserts did not establish
an advantage over six/four.

Larger batches or bundles and a second sampler core did not beat the selected
settings in the screening trials. Increasing database affinity from 15 to 31 cores
also did not help the targeted 16-evaluator comparison. PostgreSQL used roughly
3–4 cores there. Input COPY, WAL and buffer/write waits remain useful next profiling
targets; the I/O busy fraction alone does not identify a hardware bandwidth limit.

A 4GB PostgreSQL cache reduced sampled data-file-write waits and raised the observed
rates further. The large-host frontier preset now uses that cache size and records
it explicitly. General deployments retain the 256MB memory default; see
[capacity planning](operations.md#capacity-planning) for the high-throughput setting.
Old benchmark suites without the new resource fields replay with two sampler
connections and 256MB, and comparisons reject differing resource settings.

Two independent deployments with disjoint CPU allocations achieved **7.38M/s
aggregate** (3.63M/s + 3.76M/s), versus 4.34M/s for one deployment on its own:
1.70× aggregate throughput using two samplers, two databases and twice the allocated
CPU/cache resources. These were barrier-aligned 40-second materialized measurements
with 16 evaluators and 4GB PostgreSQL shared buffers per instance, sharing the host
and storage. Independent deployments already support this scaling; a single
adaptive run still has one sampler and one ordered feedback stream.

Regression controls found no loss: tiny 256-sample batches remained around 0.34M/s,
and a 50-µs training workload remained around 0.316M/s (16 evaluators; ideal 0.32M/s).
RNG inference remained around 30M/s. These checks compare the new configuration
with the earlier benchmark settings or, for the small/delayed controls, the old
run default of two connections and eight inserts.

The initial stress study exposed the aborted-cleanup row-lock race fixed above.
Its failed attempt is retained in the development evidence. Two initial CPU-affinity
probes did not reach the PostgreSQL daemon and are excluded; corrected probes were
run afterward. All subsequent studies, including an eight-insert cleanup stress
case, shut down cleanly. The 348 Rust unit/binary tests, 37 PostgreSQL integration
tests and 52 benchmark harness tests passed. See the
[portable trial data and provenance](benchmarks/2026-09-30/summary.json).
The production Rust/Python process adapters are unchanged by this tuning.

## Generation baseline — 2026-09-29

The sampler-generation implementation was measured with the then-current sparse
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

Other focused follow-ups remain separate: GammaLoop entry-reset cost, finite
training windows/MadNIS behavior, and dependency numerical stability. An earlier
Havana fixture exposed tiny negative rounded variance for identical values of
1/6; varying benchmark feedback avoids that fixture but is not its numerical fix.
