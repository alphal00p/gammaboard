CREATE TABLE IF NOT EXISTS run_stage_snapshots (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT REFERENCES run_tasks(id) ON DELETE SET NULL,
    sequence_nr INT,
    queue_empty BOOLEAN NOT NULL,
    sampler_snapshot JSONB NOT NULL,
    observable_state JSONB,
    sampler_aggregator JSONB NOT NULL,
    materializer JSONB NOT NULL,
    batch_transforms JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_run_stage_snapshots_run_created
    ON run_stage_snapshots(run_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_run_stage_snapshots_run_sequence
    ON run_stage_snapshots(run_id, sequence_nr, created_at DESC);

ALTER TABLE run_tasks
    ADD CONSTRAINT run_tasks_spawned_from_snapshot_fkey
    FOREIGN KEY (spawned_from_snapshot_id) REFERENCES run_stage_snapshots(id) ON DELETE SET NULL;
