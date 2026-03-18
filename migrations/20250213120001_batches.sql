-- Batches: work queue for distributed evaluation
-- Each batch contains multiple samples that get evaluated together
CREATE TABLE IF NOT EXISTS batches (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,

    -- Versioned latent queue payload that evaluators materialize locally.
    latent_batch JSONB NOT NULL,
    parametrization_state_version BIGINT NOT NULL,

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
    -- Batch-level observable payload emitted by the evaluator runner

    total_eval_time_ms DOUBLE PRECISION,
    -- Total time to evaluate all samples in batch

    -- For retry logic
    retry_count INT DEFAULT 0,
    last_error TEXT
);

CREATE TABLE IF NOT EXISTS parametrization_states (
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    version BIGINT NOT NULL,
    -- Canonical persisted parametrization state payload: { config, snapshot }.
    state JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, version)
);

CREATE INDEX IF NOT EXISTS idx_parametrization_states_run_created
    ON parametrization_states(run_id, created_at DESC);

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
