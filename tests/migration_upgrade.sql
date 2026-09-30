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

-- A deployment can contain both old claims and retained completed retry rows.
INSERT INTO batches (run_id, task_id, batch_size, status, retry_count, claimed_by_node_uuid)
SELECT run_id, id, 250, 'claimed', 0, 'upgrade-worker-uuid'
FROM run_tasks WHERE name='active-task';
INSERT INTO batches (run_id, task_id, batch_size, status, retry_count)
SELECT run_id, id, 250, 'completed', 1
FROM run_tasks WHERE name='active-task';
INSERT INTO batch_results (batch_id, batch_observable, completed_at)
SELECT id, '{"empty":{}}', now() FROM batches WHERE status='completed';

\ir ../migrations/202609230001_batch_claim_tokens.sql
DO $$
BEGIN
    IF (SELECT count(*) FROM batches WHERE claim_token IS NULL) <> 2
       OR (SELECT count(*) FROM batch_results) <> 1 THEN
        RAISE EXCEPTION 'claim-token migration changed existing batches or results';
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


INSERT INTO runs (name, point_spec, parent_run_id)
SELECT 'upgrade-child', point_spec, id FROM runs WHERE name='upgrade-survivor';
UPDATE nodes SET desired_run_id=(SELECT id FROM runs WHERE name='upgrade-child'),
                 desired_role='evaluator'
WHERE name='upgrade-worker';
\ir ../migrations/202609170001_worker_pools.sql
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM nodes n JOIN runs p ON p.id=n.pool_run_id
        JOIN runs c ON c.id=n.desired_run_id
        WHERE n.name='upgrade-worker' AND p.name='upgrade-survivor'
          AND c.name='upgrade-child' AND n.pool_role='evaluator'
    ) THEN RAISE EXCEPTION 'worker pool migration lost ownership or placement'; END IF;
END $$;

CREATE TEMP TABLE upgrade_counter_before AS TABLE run_batch_queue_counters;
\ir ../migrations/202609240001_defer_queue_counters.sql
DO $$
BEGIN
    IF EXISTS (TABLE run_batch_queue_counters EXCEPT TABLE upgrade_counter_before)
       OR EXISTS (TABLE upgrade_counter_before EXCEPT TABLE run_batch_queue_counters) THEN
        RAISE EXCEPTION 'deferred-counter migration changed existing counters';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_trigger WHERE tgname='batches_queue_counter_trigger'
                   AND tgdeferrable AND tginitdeferred) THEN
        RAISE EXCEPTION 'queue counter trigger is not deferred until commit';
    END IF;
END $$;

\ir ../migrations/202609240002_input_compression.sql

CREATE TEMP TABLE upgrade_cpu_before AS SELECT id, cpu_seconds FROM run_tasks;
DO $$
DECLARE telemetry_table TEXT;
BEGIN
    FOREACH telemetry_table IN ARRAY ARRAY[
        'evaluator_performance_history', 'evaluator_performance_latest',
        'sampler_aggregator_performance_history', 'sampler_aggregator_performance_latest'
    ] LOOP
        EXECUTE format('INSERT INTO %I (id,run_id,worker_id)
                        SELECT 0,id,''upgrade-worker'' FROM runs WHERE name=''upgrade-survivor''', telemetry_table);
    END LOOP;
END $$;
\ir ../migrations/202609290001_worker_accounting.sql
\ir ../migrations/202609290002_telemetry_ownership.sql
UPDATE nodes SET lease_expires_at=now()+interval '1 hour' WHERE name='upgrade-worker';
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM run_tasks t JOIN upgrade_cpu_before b USING(id)
               WHERE t.cpu_seconds<>b.cpu_seconds OR task_cpu_seconds(t.id,t.cpu_seconds)<b.cpu_seconds) THEN
        RAISE EXCEPTION 'worker accounting migration lost existing task totals';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM task_worker_cpu_time WHERE cpu_seconds>0) THEN
        RAISE EXCEPTION 'upgraded worker did not write independent accounting';
    END IF;
    IF EXISTS (SELECT run_id,worker_id FROM evaluator_performance_history
               EXCEPT SELECT run_id,worker_id FROM run_telemetry_workers) THEN
        RAISE EXCEPTION 'telemetry migration lost run ownership';
    END IF;
END $$;
