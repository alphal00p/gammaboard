# Worker concurrency and busy measurements

GammaBoard keeps model and accumulator mutation serial within each worker,
while allowing independent workers and bounded database I/O to overlap.

| Scope | Serial work | Allowed overlap |
| --- | --- | --- |
| Evaluator worker | Materialize and evaluate one batch at a time using one evaluator instance. | One next-batch fetch and one previous-result submission can run alongside the current evaluation. |
| Sampler/aggregator worker | Generate samples, ingest returned training values/update the model, and merge accumulators through one mutable sampler/accumulator owner. | Queue inserts, one completed-result fetch, one aggregation write, and one cleanup operation can run in the background. Insert concurrency is bounded by `max_concurrent_insert_tasks` and the DB connection pool. |
| Worker process | Main calling thread drives its active role. | One Tokio background thread, by default, handles asynchronous I/O, leases, and task-control polling. Several waiting I/O tasks share that thread. |
| Run | One active sampler owns each sampling task. | Evaluator processes evaluate different batches concurrently. Controller runs may distribute a worker pool across child runs within their configured limits. |

The API server retains its automatic Tokio thread pool. `TOKIO_WORKER_THREADS`
overrides the worker default. Libraries and external processes may have their
own CPU/GPU parallelism; the MADNIS launcher defaults its OpenMP threads to one.
One I/O thread per worker does not mean all computation across the deployment
is single-threaded.

## Training and inference

During a training minibatch, the sampler produces the allowed number of samples
and evaluators process them in parallel. Once that training window is exhausted,
generation pauses until the needed results return. Training updates and new
draws use the same mutable sampler and cannot run concurrently. With bulk
generation, one draw is split into evaluator batches and its returned training
values are collected before the corresponding ingestion. Heartbeats and
independent database I/O can continue during this work.

During a model update, evaluator busy can fall because the next samples are not
available yet. In inference, there is no training barrier, so generation,
evaluation on other processes, and result persistence can run as a pipeline.
The prefetch/submission slots hide database latency; they do not permit two
batches to mutate one evaluator concurrently.

Final checkpoint writes wait for previous writes and drain outstanding work.
An older background write must not overwrite a newer final checkpoint. Claim
tokens fence stale evaluator submissions after reassignment.

## What changed

The earlier simplification removed machine-sized background thread pools from
each worker and the extra post-batch GammaLoop reset. GammaLoop still restores
its pristine integrand before each observable batch, including after a failure.
The bounded prefetch, submission, and persistence overlap remains. No new
compute parallelism was added for the graphs.

## Four activity rates

| Role | Compute busy | I/O active |
| --- | --- | --- |
| Evaluators | Materialize and evaluate | Claim/fetch batches, submit results, reconcile claims |
| Sampler | Generate, train/ingest, merge | Queue queries, insert/fetch, persistence, cleanup |

For either column: `100 × occupied seconds / elapsed seconds`, measured with
one monotonic clock per worker. Concurrent operations within a column count
once; compute and I/O may overlap. I/O begins when the operation starts executing
and ends when it returns, fails, or is cancelled. Waiting for a DB connection or
query is included. Queued-but-unstarted tasks and completed-but-uncollected
results are excluded. Heartbeats, control polling, and telemetry are excluded.
Counters include open operations at the instant of a snapshot. Differences of
cumulative counters survive skipped publications without losing work.

These are activity fractions, not thread CPU or database utilization. High
compute on one role with low compute on the other suggests a compute bottleneck.
High I/O with low compute suggests investigating data movement; high I/O alone
can simply mean useful overlap. Low activity can be legitimate starvation,
backpressure, a training barrier, or pacing. Use accepted throughput and the
Diagnostics queue/timing details to distinguish those cases.

The graph has four stable traces, averaged over measured worker-time in at most
600 display intervals for the selected range. Its linked sliders can browse the
full recorded history and request finer detail on zoom. No min/max overlay or
smoothing is applied. Current overview rates require fresh complete worker
coverage. Historical fleet means cover reporting workers; missing data is not
zero. Long synchronous calls publish only after completion, even if the configured
snapshot interval is shorter. The graph reports observed spacing separately from
its display-bin width.

## I/O limits and scheduling

Both role DB pools default to two connections, matching the existing two-
connection worker cap. A requested size of one is supported. Legacy values
outside 1–2 are clamped with a warning; the config panel shows the effective size.
Control-plane connections remain separate so leases can progress during role I/O.

One bounded insert pump handles both enqueue and refill. Completed inserts are
collected before refill, avoiding recursive scheduling from each result handler.
Result fetches still respect outstanding insert boundaries to prevent cursor
advancement past uncommitted earlier batches. Persistence measures operation
execution, not time until its handle is collected. Final writes retain their
ordering and checkpoint-aware cleanup remains intact.

Queue-counter updates are deferred until transaction commit, so large input
copies do not hold the shared per-run counter lock throughout the transfer.
Payload serialization precedes connection acquisition and borrows its arrays;
the counter, batch metadata, and inputs still become visible atomically.
New input writes use LZ4 when supported by PostgreSQL. Insert bundle size and
concurrency limits are unchanged by these storage optimizations.

Use `scripts/benchmark.py io` to compare one, two, and eight concurrent inserts.
Its evaluator fleets share an explicit CPU budget: large fleets stress scheduling
and database contention rather than demonstrate CPU strong scaling.
