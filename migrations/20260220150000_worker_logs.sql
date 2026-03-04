-- Unified runtime logs from worker, API, and control-plane contexts.
CREATE TABLE IF NOT EXISTS runtime_logs (
    id BIGSERIAL PRIMARY KEY,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    source TEXT NOT NULL,
    run_id INT,
    node_id TEXT,
    worker_id TEXT,
    request_id TEXT,
    level TEXT NOT NULL,
    target TEXT NOT NULL,
    message TEXT NOT NULL,
    fields JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_run_id
    ON runtime_logs(source, run_id, id DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_worker_id
    ON runtime_logs(source, worker_id, id DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_node_id
    ON runtime_logs(source, node_id, id DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_level
    ON runtime_logs(source, level, id DESC);
