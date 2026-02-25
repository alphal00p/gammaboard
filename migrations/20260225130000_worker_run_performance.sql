-- Worker performance history (append-only).
-- Split by role so evaluator and sampler-aggregator can evolve independently.

CREATE TABLE IF NOT EXISTS evaluator_performance_history (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES workers(worker_id) ON DELETE CASCADE,
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,
    batches_completed BIGINT NOT NULL,
    samples_evaluated BIGINT NOT NULL,
    avg_time_per_sample_ms DOUBLE PRECISION NOT NULL,
    std_time_per_sample_ms DOUBLE PRECISION NOT NULL,
    diagnostics JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_eval_perf_history_run_worker_time
    ON evaluator_performance_history(run_id, worker_id, window_end DESC);

CREATE INDEX IF NOT EXISTS idx_eval_perf_history_run_time
    ON evaluator_performance_history(run_id, window_end DESC);

CREATE OR REPLACE VIEW evaluator_performance_latest AS
SELECT DISTINCT ON (run_id, worker_id)
    id,
    run_id,
    worker_id,
    window_start,
    window_end,
    batches_completed,
    samples_evaluated,
    avg_time_per_sample_ms,
    std_time_per_sample_ms,
    diagnostics,
    created_at
FROM evaluator_performance_history
ORDER BY run_id, worker_id, window_end DESC, id DESC;

CREATE TABLE IF NOT EXISTS sampler_aggregator_performance_history (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES workers(worker_id) ON DELETE CASCADE,
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,
    produced_batches BIGINT NOT NULL,
    produced_samples BIGINT NOT NULL,
    avg_produce_time_per_sample_ms DOUBLE PRECISION NOT NULL,
    std_produce_time_per_sample_ms DOUBLE PRECISION NOT NULL,
    ingested_batches BIGINT NOT NULL,
    ingested_samples BIGINT NOT NULL,
    avg_ingest_time_per_sample_ms DOUBLE PRECISION NOT NULL,
    std_ingest_time_per_sample_ms DOUBLE PRECISION NOT NULL,
    diagnostics JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sampler_perf_history_run_worker_time
    ON sampler_aggregator_performance_history(run_id, worker_id, window_end DESC);

CREATE INDEX IF NOT EXISTS idx_sampler_perf_history_run_time
    ON sampler_aggregator_performance_history(run_id, window_end DESC);

CREATE OR REPLACE VIEW sampler_aggregator_performance_latest AS
SELECT DISTINCT ON (run_id, worker_id)
    id,
    run_id,
    worker_id,
    window_start,
    window_end,
    produced_batches,
    produced_samples,
    avg_produce_time_per_sample_ms,
    std_produce_time_per_sample_ms,
    ingested_batches,
    ingested_samples,
    avg_ingest_time_per_sample_ms,
    std_ingest_time_per_sample_ms,
    diagnostics,
    created_at
FROM sampler_aggregator_performance_history
ORDER BY run_id, worker_id, window_end DESC, id DESC;
