 - implement symbolica evaluator with propper building from config
 - add basic auth to dashboard which enables steering (spawning virtual workers, creating runs etc.)
 - implement gammaloop evaluator
 - implement madnis sampler_aggregator

 
 - improve logging in the dashboard (styling still needs work)

 - performance: avoid using `run_progress` (batch-grouping view) as a hot path for 1s SSE + frequent polling.
  - options: materialized/cached per-run summary table updated by runners, or pre-aggregated counters updated incrementally.
 - performance: rework SSE backend loop to avoid one DB polling loop per client.
  - use per-run fanout/broadcast task (shared polling) and lightweight client subscriptions.
 - API safety: clamp `GET /api/runs/:id/aggregated?limit=` similar to logs/perf endpoints.
 - frontend reliability/perf: prevent overlapping polling requests (`setInterval(async ...)` currently allows overlap).
  - add in-flight guards + `AbortController` on cleanup/run switch.
 - logs transport: switch from polling full latest 500 logs to incremental fetch model.
  - add cursor (`since_id`) endpoint and client-side append; optionally add log SSE delta stream.
 - histories streaming model:
  - keep REST history endpoints for backfill/initial load.
  - add optional cursor-based delta SSE for evaluator/sampler performance history (append-only rows).
