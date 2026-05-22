-- Run task queue and sample work queue.
CREATE TABLE IF NOT EXISTS run_tasks (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sequence_nr INT NOT NULL,
    task JSONB NOT NULL,
    task_toml TEXT NOT NULL,
    spawned_from_snapshot_id BIGINT,
    state TEXT NOT NULL DEFAULT 'pending',
    nr_produced_samples BIGINT NOT NULL DEFAULT 0,
    nr_completed_samples BIGINT NOT NULL DEFAULT 0,
    measurement_output JSONB,
    controller_output JSONB,
    failure_reason TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT run_tasks_state_check CHECK (
        state IN ('pending', 'active', 'completed', 'failed')
    ),
    CONSTRAINT run_tasks_progress_check CHECK (
        nr_produced_samples >= 0
        AND nr_completed_samples >= 0
        AND nr_completed_samples <= nr_produced_samples
    ),
    CONSTRAINT run_tasks_sample_stop_condition_shape CHECK (
        CASE
            WHEN task->>'kind' = 'sample' THEN
                jsonb_typeof(task->'stop_condition') = 'object'
                AND NOT (task ? 'nr_samples')
            ELSE
                NOT (task ? 'stop_condition')
                AND NOT (task ? 'nr_samples')
        END
    ),
    CONSTRAINT run_tasks_sequence_unique UNIQUE (run_id, sequence_nr),
    CONSTRAINT run_tasks_name_unique UNIQUE (run_id, name)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_run_tasks_active_one_per_run
    ON run_tasks(run_id)
    WHERE state = 'active';

CREATE INDEX IF NOT EXISTS idx_run_tasks_run_sequence
    ON run_tasks(run_id, sequence_nr, id);

CREATE INDEX IF NOT EXISTS idx_run_tasks_run_name
    ON run_tasks(run_id, name);

CREATE TABLE IF NOT EXISTS batches (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    requires_training_values BOOLEAN NOT NULL DEFAULT FALSE,
    batch_size INT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    claimed_by_node_name TEXT,
    claimed_by_node_uuid TEXT,
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT now(),
    retry_count INT DEFAULT 0,
    last_error TEXT
);

CREATE TABLE IF NOT EXISTS batch_inputs (
    batch_id BIGINT PRIMARY KEY REFERENCES batches(id) ON DELETE CASCADE,
    latent_batch BYTEA NOT NULL
);

CREATE TABLE IF NOT EXISTS batch_results (
    batch_id BIGINT PRIMARY KEY REFERENCES batches(id) ON DELETE CASCADE,
    "values" BYTEA,
    batch_observable JSONB NOT NULL,
    total_eval_time_ms DOUBLE PRECISION,
    completed_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_batches_status_runid
    ON batches(run_id, status)
    WHERE status IN ('pending', 'claimed');

CREATE INDEX IF NOT EXISTS idx_batches_pending_run_created
    ON batches(run_id, created_at, id)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_batches_task_created
    ON batches(task_id, created_at);

CREATE INDEX IF NOT EXISTS idx_batches_claimed
    ON batches(claimed_at)
    WHERE status = 'claimed';

CREATE INDEX IF NOT EXISTS idx_batches_completed
    ON batches(run_id, completed_at)
    WHERE status = 'completed';

CREATE INDEX IF NOT EXISTS idx_batches_completed_run_id
    ON batches(run_id, id)
    WHERE status = 'completed';

CREATE INDEX IF NOT EXISTS idx_batches_run_id_id
    ON batches(run_id, id);

CREATE INDEX IF NOT EXISTS idx_batch_results_completed_at
    ON batch_results(completed_at);

-- Compact task-local panel/output history.
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

CREATE INDEX IF NOT EXISTS idx_persisted_observable_snapshots_run_task_id_id_desc
    ON persisted_observable_snapshots(run_id, task_id, id DESC);

CREATE OR REPLACE VIEW work_queue_stats AS
SELECT
    b.run_id,
    b.status,
    COUNT(*) AS batch_count,
    SUM(b.batch_size) AS total_samples,
    AVG(r.total_eval_time_ms) AS avg_batch_time_ms,
    AVG(r.total_eval_time_ms / NULLIF(b.batch_size, 0)) AS avg_sample_time_ms
FROM batches b
LEFT JOIN batch_results r ON r.batch_id = b.id
GROUP BY b.run_id, b.status;
