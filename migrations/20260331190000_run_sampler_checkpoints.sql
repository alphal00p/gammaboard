CREATE TABLE IF NOT EXISTS run_sampler_checkpoints (
    run_id INT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    sampler_checkpoint JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
