# Prefetch restriction removal and remaining overhead

The batch-claim query no longer prevents prefetching while an assigned peer has
no claimed batch. There is no replacement fairness policy. Evaluator prefetch
remains a single slot. The code change removes the complete peer/age condition;
the experiment does not isolate policy-induced waiting from SQL execution cost.

## Removal experiment

Materialized native CPU workload, feedback off, approximately 5 µs/sample,
32,768 samples/batch and 131,072 samples/generation. Three paired 16-evaluator
repeats used fresh databases, fixed disjoint CPU assignments and AB/BA/AB order.
Each measured window was approximately 20 seconds. The sampler had five cores
and four I/O threads; PostgreSQL had four cores and a 1 GiB cache.

| Median measurement | Previous policy | Restriction removed |
| --- | ---: | ---: |
| Accepted samples/s | 2.818 M | 2.965 M |
| Fetch wait per batch | 6.928 ms | 1.023 ms |
| Evaluator compute busy | 90.55% | 95.75% |

Median paired throughput gain was **4.96%**, ranging from **−1.52% to +11.26%**.
Fetch waits decreased in every pair. One one-evaluator control pair changed by
only +0.027%, with no fetch wait in either variant. This supports an avoidable
multi-worker scheduling cost, but does not establish that all remaining overhead
comes from the claim policy.

[Paired results](fairness-pairs.json), [summary](fairness-summary.json),
[manifest](fairness-manifest.json).

## Targeted follow-ups after removal

Both studies used the same native workload, feedback off and generation size as
above, with fixed CPU assignments and fresh databases. Only the named setting
changed within each study; the sampler tick stayed at 10 ms.

| Change | Workload | Baseline rate | Changed rate | Median paired gain |
| --- | --- | ---: | ---: | ---: |
| Evaluator tick: 10 → 1 ms | 1 evaluator, 512 samples/batch | 45.95 k/s | 156.15 k/s | +239.86% |
| Database cores: 4 → 8 | 16 evaluators, 32,768 samples/batch | 3.063 M/s | 3.070 M/s | +0.24% |

Tick settings had two paired repeats in AB/BA order, with 12-second windows.
Gains were +241.80% and +237.91%; compute busy rose from 24.1% to 80.6%.
Database allocations had three paired repeats in AB/BA/AB order, with 20-second
windows. Gains ranged from −0.26% to +0.33%. Compute busy was approximately 99%
and fetch waits were negligible at either allocation. There is no material
database-core scaling benefit established for this CPU-work case.

The next code change evaluated was skipping the minimum tick sleep
after productive evaluator work, using the existing empty-queue backoff when
idle. The [subsequent pacing study](../evaluator-pacing/README.md) found a repeated
short-generation regression, so no tick defaults or scheduling behavior beyond
the fairness removal were retained. Materialization/domain validation takes
approximately 2.5 ms per
32,768-sample batch; reducing per-sample allocation is a secondary candidate for
very fast evaluators. Some of that conversion cost is also in the direct
reference and cannot all be counted as deployment overhead. Exposed result
submission waits were only about 0.006 ms/batch, making that path a low priority
for this workload.

In the earlier removal experiment, database input COPY operations took roughly
9.6–23.4 ms per inserted batch versus 1.7–2.0 ms for serialization. Those are
elapsed operation times, include waiting and overlap other work. Likewise,
sampler I/O busy records whether any operation is active, not CPU utilization or
the fraction of its thread pool saturated. These observations motivate profiling
the payload path for the zero-delay ceiling; they do not demonstrate a database
CPU bottleneck in the amortization case.

[Follow-up results](overhead-results.json), [summary](overhead-summary.json),
[manifest](overhead-manifest.json). All measurements used a shared host; ranges
describe observed repeats, not confidence intervals. Training and process-API
workloads were not repeated in these follow-ups.

## Validation and provenance

All 37 PostgreSQL integration tests passed on Rust 1.94, including the replacement
regression for prefetching fresh work while a peer is idle. Formatting and CI's
all-targets/all-features Clippy command with warnings denied passed on Rust 1.99.
A separate zero-delay materialized check with 512 evaluators and 32,768-sample
batches passed readiness, measurement validity and shutdown (3.39 M samples/s).
That single point validates lifecycle behavior; it is not a new frontier study.
All eight removal-experiment and ten follow-up deployments shut down cleanly.

Raw windows, run cards, experimental patch, harnesses, build logs and immutable
binaries remain in `/common/dev/cedric/setup-logs/prefetch-fairness-ab-20261001/`
and `/common/dev/cedric/setup-logs/prefetch-removal-20261001/`. The manifests record
binary hashes and CPU allocations. Both compared binaries used Rust 1.94 with
the same optimized build profile.
