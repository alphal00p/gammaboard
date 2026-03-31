-- Support incremental sampler ingestion and active evaluator counting.

CREATE INDEX IF NOT EXISTS idx_batches_run_id_id
    ON batches(run_id, id);

CREATE INDEX IF NOT EXISTS idx_nodes_active_evaluator_run
    ON nodes(active_run_id)
    WHERE active_role = 'evaluator';
