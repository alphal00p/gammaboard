-- Structured worker log events for per-run and per-worker observability.
CREATE TABLE IF NOT EXISTS worker_logs (
    id BIGSERIAL PRIMARY KEY,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    run_id INT,
    node_id TEXT,
    worker_id TEXT NOT NULL,
    role TEXT NOT NULL,
    level TEXT NOT NULL,
    event_type TEXT NOT NULL,
    message TEXT NOT NULL,
    fields JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_worker_logs_run_ts
    ON worker_logs(run_id, ts DESC);

CREATE INDEX IF NOT EXISTS idx_worker_logs_worker_ts
    ON worker_logs(worker_id, ts DESC);

CREATE INDEX IF NOT EXISTS idx_worker_logs_node_ts
    ON worker_logs(node_id, ts DESC);
