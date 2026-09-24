-- Read-only diagnostics for campaign 55. psql connection: localhost:5400, gammaboard_db.
BEGIN READ ONLY;
SET LOCAL statement_timeout = '15s';

SELECT now() AS observed_at;
SELECT id, state, task->'allocation' AS allocation,
       controller_output->'selected_child_run_ids' AS selected,
       controller_output->'running_children' AS unfinished_children,
       controller_output->'total_samples' AS samples
FROM run_tasks WHERE run_id = 55;

SELECT desired_run_id, desired_role, count(*) AS stored_assignments,
       count(*) FILTER (WHERE lease_expires_at > now()) AS live_assignments,
       min(last_seen), max(last_seen)
FROM nodes WHERE pool_run_id = 55
GROUP BY 1, 2 ORDER BY 1, 2;

SELECT name, desired_run_id, active_run_id, active_role, last_seen, activity
FROM nodes WHERE pool_run_id = 55 AND lease_expires_at > now()
AND pool_role = 'sampler_aggregator';

SELECT run_id, task_id, updated_at, md5(sampler_checkpoint::text) AS checkpoint_hash,
       sampler_checkpoint->'completed_samples' AS completed,
       sampler_checkpoint->'runtime_state'->'produced_samples_total' AS produced,
       sampler_checkpoint->'queue' AS queue
FROM run_sampler_checkpoints WHERE run_id = 56;

SELECT b.task_id, b.status, count(*) AS batches, sum(b.batch_size) AS samples,
       count(br.batch_id) AS stored_results
FROM batches b LEFT JOIN batch_results br ON br.batch_id = b.id
WHERE b.run_id = 56 GROUP BY 1, 2;

SELECT ts, run_id, message, fields
FROM runtime_logs WHERE run_id = 56
AND fields->>'error' LIKE '%recovery checkpoint changed during runtime initialization%'
ORDER BY id DESC LIMIT 10;

SELECT '7553746958286027000'::jsonb = '7.553746958286027e18'::jsonb
    AS same_postgres_number;
SELECT current_setting('max_connections') AS max_connections,
       count(*) FILTER (WHERE backend_type = 'client backend') AS client_connections
FROM pg_stat_activity;
ROLLBACK;
