-- Worker performance history (append-only).
-- Split by role so evaluator and sampler-aggregator can evolve independently.

CREATE TABLE IF NOT EXISTS evaluator_performance_history (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES workers(worker_id) ON DELETE CASCADE,
    metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_eval_perf_history_run_worker_time
    ON evaluator_performance_history(run_id, worker_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_eval_perf_history_run_time
    ON evaluator_performance_history(run_id, created_at DESC);

CREATE OR REPLACE VIEW evaluator_performance_latest AS
SELECT DISTINCT ON (run_id, worker_id)
    id,
    run_id,
    worker_id,
    metrics,
    created_at
FROM evaluator_performance_history
ORDER BY run_id, worker_id, created_at DESC, id DESC;

CREATE TABLE IF NOT EXISTS sampler_aggregator_performance_history (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES workers(worker_id) ON DELETE CASCADE,
    metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    runtime_metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    engine_diagnostics JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sampler_perf_history_run_worker_time
    ON sampler_aggregator_performance_history(run_id, worker_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_sampler_perf_history_run_time
    ON sampler_aggregator_performance_history(run_id, created_at DESC);

CREATE OR REPLACE VIEW sampler_aggregator_performance_latest AS
SELECT DISTINCT ON (run_id, worker_id)
    id,
    run_id,
    worker_id,
    metrics,
    runtime_metrics,
    engine_diagnostics,
    created_at
FROM sampler_aggregator_performance_history
ORDER BY run_id, worker_id, created_at DESC, id DESC;
