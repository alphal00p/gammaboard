-- Aggregated results: periodic snapshots of statistics for numerical integration with importance sampling
-- Insert a new row periodically (e.g., every N batches or every M seconds)
-- For live view: fetch latest snapshot + manually aggregate any new completed batches
CREATE TABLE IF NOT EXISTS aggregated_results (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,

    -- Basic sample statistics (cumulative up to this checkpoint)
    nr_samples BIGINT NOT NULL DEFAULT 0,
    nr_batches INT NOT NULL DEFAULT 0,

    -- Raw statistics (unweighted)
    sum DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    sum_x2 DOUBLE PRECISION NOT NULL DEFAULT 0.0,  -- sum of x^2 for variance
    sum_abs DOUBLE PRECISION NOT NULL DEFAULT 0.0,  -- sum of |x| for error estimation
    max DOUBLE PRECISION,
    min DOUBLE PRECISION,

    -- Importance sampling statistics (weighted)
    weighted_sum DOUBLE PRECISION NOT NULL DEFAULT 0.0,  -- The actual integral estimate!
    weighted_sum_x2 DOUBLE PRECISION NOT NULL DEFAULT 0.0,  -- For weighted variance
    sum_weights DOUBLE PRECISION NOT NULL DEFAULT 0.0,  -- Sum of all weights
    effective_sample_size DOUBLE PRECISION,  -- ESS = (sum w)^2 / sum(w^2), diagnostic for importance sampling quality

    -- Computed statistics
    mean DOUBLE PRECISION,  -- Simple mean of values
    variance DOUBLE PRECISION,  -- Variance of values
    std_dev DOUBLE PRECISION,  -- Standard deviation
    error_estimate DOUBLE PRECISION,  -- Statistical error estimate

    -- Spatial distribution
    histograms JSONB,
    -- e.g., {
    --   "values": {"bins": [...], "counts": [...]},
    --   "x": {"bins": [...], "counts": [...]},
    --   "weights": {"bins": [...], "counts": [...]}
    -- }

    -- Timestamp of this snapshot
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Indexes for efficient queries
CREATE INDEX IF NOT EXISTS idx_aggregated_results_run_id ON aggregated_results(run_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_aggregated_results_created ON aggregated_results(created_at);

-- Sampler states: versioned snapshots of adaptive sampler state
-- Allows tracking how the sampler learns over time and enables reproducibility
CREATE TABLE IF NOT EXISTS sampler_states (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,

    -- Versioning (monotonically increasing per run)
    version INT NOT NULL,

    -- Sampler state: learned distributions, importance regions, hyperparameters, etc.
    state JSONB NOT NULL,
    -- e.g., {
    --   "type": "vegas",
    --   "grid": [...],
    --   "importance_map": {...},
    --   "stratification": {...},
    --   "hyperparameters": {...}
    -- }

    -- Training information
    nr_samples_trained BIGINT,  -- How many samples were used to train this state
    training_error DOUBLE PRECISION,  -- Quality/loss metric for this state

    -- Metadata
    created_at TIMESTAMPTZ DEFAULT now(),

    -- Ensure unique versions per run
    UNIQUE(run_id, version)
);

-- Indexes for sampler states
CREATE INDEX IF NOT EXISTS idx_sampler_states_run_id ON sampler_states(run_id, version DESC);
CREATE INDEX IF NOT EXISTS idx_sampler_states_created ON sampler_states(created_at);
