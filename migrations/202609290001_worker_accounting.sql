-- Keep heartbeat writes independent. Existing task totals remain the baseline;
-- new time is accumulated once per worker incarnation and task.
CREATE TABLE task_worker_cpu_time (
    task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    node_uuid TEXT NOT NULL,
    cpu_seconds DOUBLE PRECISION NOT NULL
        CHECK (cpu_seconds >= 0 AND cpu_seconds < 'Infinity'::DOUBLE PRECISION),
    PRIMARY KEY (task_id, node_uuid)
);

CREATE FUNCTION task_cpu_seconds(task_id BIGINT, baseline DOUBLE PRECISION)
RETURNS DOUBLE PRECISION LANGUAGE sql STABLE AS $$
    SELECT baseline + COALESCE(SUM(cpu_seconds), 0.0)
    FROM task_worker_cpu_time WHERE task_worker_cpu_time.task_id = $1
$$;

CREATE OR REPLACE FUNCTION account_node_cpu_time() RETURNS trigger AS $$
DECLARE
    accounted_until TIMESTAMPTZ;
    elapsed_seconds DOUBLE PRECISION;
    cpu_count DOUBLE PRECISION;
BEGIN
    accounted_until := LEAST(clock_timestamp(), OLD.lease_expires_at);
    elapsed_seconds := GREATEST(EXTRACT(EPOCH FROM accounted_until - OLD.cpu_time_accounted_at), 0.0);
    cpu_count := CASE WHEN jsonb_typeof(OLD.capabilities->'cpus') = 'number'
        THEN GREATEST((OLD.capabilities->>'cpus')::DOUBLE PRECISION, 1.0) ELSE 1.0 END;
    IF OLD.active_run_id IS NOT NULL AND elapsed_seconds > 0 THEN
        INSERT INTO task_worker_cpu_time (task_id, node_uuid, cpu_seconds)
        SELECT id, OLD.uuid,
            GREATEST(EXTRACT(EPOCH FROM accounted_until - GREATEST(OLD.cpu_time_accounted_at, started_at)), 0.0) * cpu_count
        FROM run_tasks WHERE run_id = OLD.active_run_id AND state = 'active'
        ON CONFLICT (task_id, node_uuid) DO UPDATE
        SET cpu_seconds = task_worker_cpu_time.cpu_seconds + EXCLUDED.cpu_seconds;
    END IF;
    NEW.cpu_time_accounted_at := clock_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER nodes_account_cpu_time ON nodes;
CREATE TRIGGER nodes_account_cpu_time
BEFORE UPDATE OF lease_expires_at, active_run_id, active_role, capabilities, cpu_time_accounted_at ON nodes
FOR EACH ROW EXECUTE FUNCTION account_node_cpu_time();
