ALTER TABLE runtime_logs
    RENAME COLUMN node_id TO node_uuid;

ALTER TABLE runtime_logs
    RENAME COLUMN worker_id TO node_name;

DROP INDEX IF EXISTS idx_runtime_logs_source_worker_id;
DROP INDEX IF EXISTS idx_runtime_logs_source_node_id;

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_node_name
    ON runtime_logs(source, node_name, id DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_node_uuid
    ON runtime_logs(source, node_uuid, id DESC);

WITH duplicate_roots AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY run_id
            ORDER BY id ASC
        ) AS rank_in_run
    FROM run_stage_snapshots
    WHERE queue_empty = TRUE
      AND task_id IS NULL
      AND sequence_nr = 0
)
DELETE FROM run_stage_snapshots snapshots
USING duplicate_roots roots
WHERE snapshots.id = roots.id
  AND roots.rank_in_run > 1;

CREATE UNIQUE INDEX IF NOT EXISTS uq_run_stage_snapshots_root
    ON run_stage_snapshots(run_id)
    WHERE queue_empty = TRUE
      AND task_id IS NULL
      AND sequence_nr = 0;
