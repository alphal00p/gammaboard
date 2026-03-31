-- Support task output history polling ordered by id DESC.
-- The existing (run_id, task_id, created_at DESC) index does not match the
-- current read path, which filters by run/task and paginates by snapshot id.
CREATE INDEX IF NOT EXISTS idx_persisted_observable_snapshots_run_task_id_id_desc
    ON persisted_observable_snapshots(run_id, task_id, id DESC);
