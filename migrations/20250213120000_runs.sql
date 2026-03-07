-- Runs: integration runs with adaptive sampling
CREATE TABLE IF NOT EXISTS runs (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    started_at TIMESTAMPTZ DEFAULT now(),
    completed_at TIMESTAMPTZ,
    training_completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'pending',
    -- Status: 'pending', 'warm-up', 'running', 'completed', 'paused', 'cancelled'

    -- Per-run engine and runner configuration (TOML/JSON payload)
    integration_params JSONB,
    target JSONB,
    evaluator_init_metadata JSONB,
    sampler_aggregator_init_metadata JSONB,
    point_spec JSONB NOT NULL DEFAULT '{"continuous_dims": 1, "discrete_dims": 0}'::jsonb,

    -- Summary statistics (updated periodically)
    total_batches_planned INT,
    batches_completed INT DEFAULT 0
);

-- Index for filtering by status
CREATE INDEX IF NOT EXISTS idx_runs_status ON runs(status);

-- Index for time-based queries
CREATE INDEX IF NOT EXISTS idx_runs_started_at ON runs(started_at DESC);
