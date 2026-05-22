-- Runs are the top-level integration records.
CREATE TABLE IF NOT EXISTS runs (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    started_at TIMESTAMPTZ DEFAULT now(),
    completed_at TIMESTAMPTZ,
    nr_produced_samples BIGINT NOT NULL DEFAULT 0,
    nr_completed_samples BIGINT NOT NULL DEFAULT 0,
    integration_params JSONB,
    target JSONB,
    point_spec JSONB NOT NULL,
    current_observable JSONB,
    batches_completed INT DEFAULT 0,
    exposed_info JSONB NOT NULL DEFAULT '{}'::jsonb,
    run_toml TEXT,
    sampler_runner_uptime_ms DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    CONSTRAINT runs_sample_progress_check CHECK (
        nr_produced_samples >= 0
        AND nr_completed_samples >= 0
        AND nr_completed_samples <= nr_produced_samples
    ),
    CONSTRAINT runs_sampler_runner_uptime_check CHECK (
        sampler_runner_uptime_ms >= 0.0
    )
);

CREATE INDEX IF NOT EXISTS idx_runs_started_at
    ON runs(started_at DESC);
