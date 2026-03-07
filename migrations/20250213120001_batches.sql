-- Batches: work queue for distributed evaluation
-- Each batch contains multiple samples that get evaluated together
CREATE TABLE IF NOT EXISTS batches (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,

    -- Batch data: compact row-major flat arrays + explicit 2D shape metadata
    points JSONB NOT NULL,
    -- e.g., {"continuous_rows":2, "continuous_cols":2, "continuous_data":[0.5,0.3,0.6,0.4],
    --        "discrete_rows":2, "discrete_cols":1, "discrete_data":[1,2],
    --        "weights_data":[1.0,1.0]}

    batch_size INT NOT NULL,
    -- Number of samples in this batch

    requires_training BOOLEAN NOT NULL DEFAULT true,
    -- Whether evaluator should emit per-sample training values for this batch

    -- Work queue status
    status TEXT NOT NULL DEFAULT 'pending',
    -- Status: 'pending', 'claimed', 'completed', 'failed'

    claimed_by TEXT,
    -- Worker ID that claimed this batch

    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT now(),

    -- Evaluator values used for sampler training (null until completed)
    "values" JSONB,

    batch_observable JSONB,
    -- Batch-level aggregated observable snapshot emitted by evaluator runner

    total_eval_time_ms DOUBLE PRECISION,
    -- Total time to evaluate all samples in batch

    -- For retry logic
    retry_count INT DEFAULT 0,
    last_error TEXT
);

-- Indexes for work queue pattern (critical for performance)
CREATE INDEX IF NOT EXISTS idx_batches_status_runid ON batches(run_id, status)
    WHERE status IN ('pending', 'claimed');

CREATE INDEX IF NOT EXISTS idx_batches_claimed ON batches(claimed_at)
    WHERE status = 'claimed';

CREATE INDEX IF NOT EXISTS idx_batches_completed ON batches(run_id, completed_at)
    WHERE status = 'completed';

-- View for monitoring work queue
CREATE OR REPLACE VIEW work_queue_stats AS
SELECT
    run_id,
    status,
    COUNT(*) as batch_count,
    SUM(batch_size) as total_samples,
    AVG(total_eval_time_ms) as avg_batch_time_ms,
    AVG(total_eval_time_ms / NULLIF(batch_size, 0)) as avg_sample_time_ms
FROM batches
GROUP BY run_id, status;

-- View for run progress
CREATE OR REPLACE VIEW run_progress AS
SELECT
    r.id as run_id,
    r.name as run_name,
    r.status as run_status,
    COALESCE(r.integration_params, '{}'::jsonb) as integration_params,
    r.target,
    r.evaluator_init_metadata,
    r.sampler_aggregator_init_metadata,
    r.started_at,
    r.completed_at,
    r.training_completed_at,
    r.total_batches_planned,
    r.batches_completed,
    COALESCE(b.total_batches, 0) as total_batches,
    COALESCE(b.total_samples, 0) as total_samples,
    COALESCE(b.pending_batches, 0) as pending_batches,
    COALESCE(b.claimed_batches, 0) as claimed_batches,
    COALESCE(b.completed_batches, 0) as completed_batches,
    COALESCE(b.failed_batches, 0) as failed_batches,
    CASE
        WHEN COALESCE(b.total_batches, 0) > 0
        THEN CAST(COALESCE(b.completed_batches, 0) AS FLOAT) / b.total_batches
        ELSE 0.0
    END as completion_rate
FROM runs r
LEFT JOIN (
    SELECT
        run_id,
        COUNT(*) as total_batches,
        SUM(batch_size) as total_samples,
        SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending_batches,
        SUM(CASE WHEN status = 'claimed' THEN 1 ELSE 0 END) as claimed_batches,
        SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed_batches,
        SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed_batches
    FROM batches
    GROUP BY run_id
) b ON r.id = b.run_id;
