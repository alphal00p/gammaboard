-- Aggregated results: periodic run-level snapshots of the aggregated observable.
CREATE TABLE IF NOT EXISTS aggregated_results (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    aggregated_observable JSONB NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_aggregated_results_run_id
    ON aggregated_results(run_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_aggregated_results_created
    ON aggregated_results(created_at);

-- Sampler states: versioned snapshots of adaptive sampler state.
CREATE TABLE IF NOT EXISTS sampler_states (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    version INT NOT NULL,
    state JSONB NOT NULL,
    nr_samples_trained BIGINT,
    training_error DOUBLE PRECISION,
    created_at TIMESTAMPTZ DEFAULT now(),
    UNIQUE(run_id, version)
);

CREATE INDEX IF NOT EXISTS idx_sampler_states_run_id
    ON sampler_states(run_id, version DESC);

CREATE INDEX IF NOT EXISTS idx_sampler_states_created
    ON sampler_states(created_at);
