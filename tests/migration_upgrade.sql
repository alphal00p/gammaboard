INSERT INTO runs (name, point_spec)
VALUES ('upgrade-survivor', '{"continuous": {"dims": 1}}');

INSERT INTO run_tasks (run_id, name, sequence_nr, task, task_toml, state)
SELECT id, 'active-task', 0, '{"kind": "set_accumulator", "accumulator": {"kind": "scalar"}}', '', 'active'
FROM runs WHERE name = 'upgrade-survivor';

INSERT INTO nodes (name, uuid, lease_expires_at, active_run_id, active_role, capabilities)
SELECT 'upgrade-worker', 'upgrade-worker-uuid', now() + interval '1 hour', id, 'evaluator', '{"cpus": 2}'
FROM runs WHERE name = 'upgrade-survivor';

\ir ../migrations/202609080001_task_cpu_time.sql

DO $$
DECLARE
    preserved_count INTEGER;
BEGIN
    SELECT count(*) INTO preserved_count FROM runs WHERE name = 'upgrade-survivor';
    IF preserved_count <> 1 THEN
        RAISE EXCEPTION 'upgrade did not preserve the existing run';
    END IF;
END $$;

ALTER TABLE nodes DISABLE TRIGGER nodes_account_cpu_time;
UPDATE nodes
SET cpu_time_accounted_at = now() - interval '2 seconds'
WHERE name = 'upgrade-worker';
ALTER TABLE nodes ENABLE TRIGGER nodes_account_cpu_time;

UPDATE nodes SET last_seen = now() WHERE name = 'upgrade-worker';

DO $$
DECLARE
    accounted DOUBLE PRECISION;
BEGIN
    SELECT cpu_seconds INTO accounted
    FROM run_tasks
    WHERE name = 'active-task';
    IF accounted < 3.0 THEN
        RAISE EXCEPTION 'CPU-time trigger did not account the upgraded node: %', accounted;
    END IF;
END $$;
