ALTER TABLE run_tasks
    ADD COLUMN name TEXT;

UPDATE run_tasks
SET name = CONCAT('task-', sequence_nr)
WHERE name IS NULL;

ALTER TABLE run_tasks
    ALTER COLUMN name SET NOT NULL;

ALTER TABLE run_tasks
    ADD CONSTRAINT run_tasks_name_unique UNIQUE (run_id, name);

CREATE INDEX IF NOT EXISTS idx_run_tasks_run_name
    ON run_tasks(run_id, name);

ALTER TABLE run_stage_snapshots
    ADD COLUMN name TEXT;

UPDATE run_stage_snapshots
SET name = 'root'
WHERE name IS NULL
  AND task_id IS NULL;

UPDATE run_stage_snapshots AS snapshots
SET name = tasks.name
FROM run_tasks AS tasks
WHERE snapshots.name IS NULL
  AND snapshots.task_id = tasks.id;

UPDATE run_stage_snapshots
SET name = CONCAT('task-', task_id::text)
WHERE name IS NULL
  AND task_id IS NOT NULL;

ALTER TABLE run_stage_snapshots
    ALTER COLUMN name SET NOT NULL;
