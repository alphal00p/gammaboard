-- Persisted observable snapshots: periodic task-local snapshots of the compact,
-- history-facing observable payload. This intentionally stores the persisted
-- form, not the full in-memory observable state.
CREATE TABLE IF NOT EXISTS persisted_observable_snapshots (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT NOT NULL,
    persisted_observable JSONB NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_persisted_observable_snapshots_run_task_id
    ON persisted_observable_snapshots(run_id, task_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_persisted_observable_snapshots_created
    ON persisted_observable_snapshots(created_at);
