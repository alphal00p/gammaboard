-- Runs: integration runs with adaptive sampling
CREATE TABLE IF NOT EXISTS runs (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    started_at TIMESTAMPTZ DEFAULT now(),
    completed_at TIMESTAMPTZ,
    nr_produced_samples BIGINT NOT NULL DEFAULT 0,
    nr_completed_samples BIGINT NOT NULL DEFAULT 0,

    -- Per-run engine and runner configuration (TOML/JSON payload)
    integration_params JSONB,
    target JSONB,
    point_spec JSONB NOT NULL,
    current_observable JSONB,
    sampler_runner_snapshot JSONB,

    -- Summary statistics (updated periodically)
    batches_completed INT DEFAULT 0,
    CONSTRAINT runs_sample_progress_check CHECK (
        nr_produced_samples >= 0
        AND nr_completed_samples >= 0
        AND nr_completed_samples <= nr_produced_samples
    )
);

-- Index for time-based queries
CREATE INDEX IF NOT EXISTS idx_runs_started_at ON runs(started_at DESC);
