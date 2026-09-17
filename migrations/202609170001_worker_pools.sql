-- Pool membership is operator intent; desired/active assignments are placement.
ALTER TABLE nodes ADD COLUMN pool_run_id INT REFERENCES runs(id) ON DELETE SET NULL;
ALTER TABLE nodes ADD COLUMN pool_role TEXT CHECK (pool_role IN ('evaluator', 'sampler_aggregator'));
ALTER TABLE nodes ADD CONSTRAINT nodes_pool_assignment_pair_check CHECK (
    (pool_run_id IS NULL) = (pool_role IS NULL)
);

CREATE FUNCTION worker_pool_root(run_id INT) RETURNS INT
LANGUAGE SQL STABLE AS $$
    WITH RECURSIVE ancestors AS (
        SELECT id, parent_run_id FROM runs WHERE id = run_id
        UNION
        SELECT r.id, r.parent_run_id FROM runs r JOIN ancestors a ON r.id = a.parent_run_id
    ) SELECT id FROM ancestors WHERE parent_run_id IS NULL
$$;

UPDATE nodes SET pool_run_id = worker_pool_root(desired_run_id), pool_role = desired_role
WHERE desired_run_id IS NOT NULL;
CREATE INDEX idx_nodes_pool_run ON nodes(pool_run_id);

-- Controller pools can hold multiple samplers. Execution remains exclusive
-- through idx_nodes_current_sampler_run; the scheduler chooses placements.
DROP INDEX idx_nodes_desired_sampler_run;
CREATE INDEX idx_nodes_desired_sampler_run ON nodes(desired_run_id)
WHERE desired_role = 'sampler_aggregator';
