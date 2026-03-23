CREATE TABLE IF NOT EXISTS run_tasks (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    sequence_nr INT NOT NULL,
    task JSONB NOT NULL,
    spawned_from_snapshot_id BIGINT,
    state TEXT NOT NULL DEFAULT 'pending',
    nr_produced_samples BIGINT NOT NULL DEFAULT 0,
    nr_completed_samples BIGINT NOT NULL DEFAULT 0,
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
    CONSTRAINT run_tasks_sequence_unique UNIQUE (run_id, sequence_nr)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_run_tasks_active_one_per_run
    ON run_tasks(run_id)
    WHERE state = 'active';

CREATE INDEX IF NOT EXISTS idx_run_tasks_run_sequence
    ON run_tasks(run_id, sequence_nr);

ALTER TABLE batches
    ADD CONSTRAINT batches_task_id_fkey
    FOREIGN KEY (task_id) REFERENCES run_tasks(id) ON DELETE CASCADE;
