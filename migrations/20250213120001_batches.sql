-- Batches: work queue for distributed evaluation
-- Each batch contains multiple samples that get evaluated together
CREATE TABLE IF NOT EXISTS batches (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT NOT NULL,
    requires_training_values BOOLEAN NOT NULL DEFAULT FALSE,

    batch_size INT NOT NULL,
    -- Number of samples in this batch

    -- Work queue status
    status TEXT NOT NULL DEFAULT 'pending',
    -- Status: 'pending', 'claimed', 'completed', 'failed'

    claimed_by_node_name TEXT,
    claimed_by_node_uuid TEXT,
    -- Live node owner that claimed this batch

    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT now(),

    -- For retry logic
    retry_count INT DEFAULT 0,
    last_error TEXT
);

CREATE TABLE IF NOT EXISTS batch_inputs (
    batch_id BIGINT PRIMARY KEY REFERENCES batches(id) ON DELETE CASCADE,
    latent_batch JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS batch_results (
    batch_id BIGINT PRIMARY KEY REFERENCES batches(id) ON DELETE CASCADE,
    "values" JSONB,
    batch_observable JSONB NOT NULL,
    total_eval_time_ms DOUBLE PRECISION,
    completed_at TIMESTAMPTZ NOT NULL
);

-- Indexes for work queue pattern (critical for performance)
CREATE INDEX IF NOT EXISTS idx_batches_status_runid ON batches(run_id, status)
    WHERE status IN ('pending', 'claimed');

CREATE INDEX IF NOT EXISTS idx_batches_pending_run_created
    ON batches(run_id, created_at, id)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_batches_task_created ON batches(task_id, created_at);

CREATE INDEX IF NOT EXISTS idx_batches_claimed ON batches(claimed_at)
    WHERE status = 'claimed';

CREATE INDEX IF NOT EXISTS idx_batches_completed ON batches(run_id, completed_at)
    WHERE status = 'completed';

CREATE INDEX IF NOT EXISTS idx_batches_completed_run_id
    ON batches(run_id, id)
    WHERE status = 'completed';

CREATE INDEX IF NOT EXISTS idx_batch_results_completed_at
    ON batch_results(completed_at);

-- View for monitoring work queue
CREATE OR REPLACE VIEW work_queue_stats AS
SELECT
    b.run_id,
    b.status,
    COUNT(*) as batch_count,
    SUM(b.batch_size) as total_samples,
    AVG(r.total_eval_time_ms) as avg_batch_time_ms,
    AVG(r.total_eval_time_ms / NULLIF(b.batch_size, 0)) as avg_sample_time_ms
FROM batches b
LEFT JOIN batch_results r ON r.batch_id = b.id
GROUP BY b.run_id, b.status;
