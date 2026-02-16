-- Workers: global worker registry
-- Stores information about workers (machines/processes) that evaluate batches
CREATE TABLE IF NOT EXISTS workers (
    id TEXT PRIMARY KEY,  -- Worker ID (e.g., hostname, UUID, or custom identifier)

    -- Worker information
    name TEXT,  -- Human-readable name
    specs JSONB,
    -- e.g., {
    --   "cpu": "AMD Ryzen 9 5950X",
    --   "cores": 32,
    --   "ram_gb": 64,
    --   "gpu": "NVIDIA RTX 3090",
    --   "os": "Linux 6.1.0"
    -- }

    -- Status
    last_seen TIMESTAMPTZ,
    status TEXT DEFAULT 'active',  -- active/inactive/offline

    -- Metadata
    first_seen TIMESTAMPTZ DEFAULT now(),
    created_at TIMESTAMPTZ DEFAULT now()
);

-- Worker performance: per-run statistics for each worker
-- Tracks how well each worker performs on specific runs
CREATE TABLE IF NOT EXISTS worker_performance (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES workers(id) ON DELETE CASCADE,

    -- Performance metrics
    batches_completed INT DEFAULT 0,
    samples_evaluated BIGINT DEFAULT 0,
    total_eval_time_ms DOUBLE PRECISION DEFAULT 0.0,
    avg_sample_time_ms DOUBLE PRECISION,

    -- Efficiency metrics
    batches_failed INT DEFAULT 0,
    batches_timeout INT DEFAULT 0,

    -- Time tracking
    started_at TIMESTAMPTZ DEFAULT now(),
    last_batch_at TIMESTAMPTZ,

    -- Ensure one record per worker per run
    UNIQUE(run_id, worker_id)
);

-- Indexes for worker performance
CREATE INDEX IF NOT EXISTS idx_worker_performance_run_id ON worker_performance(run_id);
CREATE INDEX IF NOT EXISTS idx_worker_performance_worker_id ON worker_performance(worker_id);
CREATE INDEX IF NOT EXISTS idx_workers_status ON workers(status, last_seen);
