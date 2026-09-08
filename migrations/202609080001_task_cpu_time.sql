ALTER TABLE run_tasks
ADD COLUMN cpu_seconds DOUBLE PRECISION NOT NULL DEFAULT 0.0
CHECK (cpu_seconds >= 0.0 AND cpu_seconds < 'Infinity'::DOUBLE PRECISION);

ALTER TABLE nodes
ADD COLUMN cpu_time_accounted_at TIMESTAMPTZ NOT NULL DEFAULT now();

-- Account allocated worker time whenever a node heartbeat or assignment update
-- advances its persisted state. The previous capability set applies to the
-- elapsed interval; nodes without an explicit CPU count represent one CPU.
CREATE FUNCTION account_node_cpu_time() RETURNS trigger AS $$
DECLARE
    accounted_until TIMESTAMPTZ;
    elapsed_seconds DOUBLE PRECISION;
    cpu_count DOUBLE PRECISION;
BEGIN
    accounted_until := LEAST(clock_timestamp(), OLD.lease_expires_at);
    elapsed_seconds := GREATEST(
        EXTRACT(EPOCH FROM accounted_until - OLD.cpu_time_accounted_at),
        0.0
    );
    cpu_count := CASE
        WHEN jsonb_typeof(OLD.capabilities->'cpus') = 'number'
            THEN GREATEST((OLD.capabilities->>'cpus')::DOUBLE PRECISION, 1.0)
        ELSE 1.0
    END;

    IF OLD.active_run_id IS NOT NULL AND elapsed_seconds > 0.0 THEN
        UPDATE run_tasks
        SET cpu_seconds = cpu_seconds + elapsed_seconds * cpu_count
        WHERE run_id = OLD.active_run_id
          AND state = 'active';
    END IF;

    NEW.cpu_time_accounted_at := clock_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER nodes_account_cpu_time
BEFORE UPDATE ON nodes
FOR EACH ROW
EXECUTE FUNCTION account_node_cpu_time();
