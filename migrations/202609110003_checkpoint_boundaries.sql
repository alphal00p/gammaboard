-- Upgrade old checkpoints only when a graceful stop left progress and queued work
-- exactly at their saved boundary. Stale/incomplete checkpoints remain unsupported.
UPDATE run_sampler_checkpoints c
SET sampler_checkpoint = jsonb_set(c.sampler_checkpoint, '{queue,last_produced_batch_id}',
    COALESCE(to_jsonb(COALESCE(
        (SELECT max(b.id) FROM batches b WHERE b.run_id=c.run_id AND b.task_id=c.task_id),
        (c.sampler_checkpoint->'queue'->>'last_completed_batch_id')::bigint
    )), 'null'::jsonb)) || jsonb_build_object(
        'output_snapshot_id', (SELECT max(id) FROM persisted_observable_snapshots WHERE run_id=c.run_id AND task_id=c.task_id),
        'batches_completed', r.batches_completed)
FROM run_tasks t, runs r
WHERE t.id=c.task_id AND r.id=c.run_id
  AND NOT (c.sampler_checkpoint->'queue' ? 'last_produced_batch_id')
  AND t.nr_produced_samples=(c.sampler_checkpoint->'runtime_state'->>'produced_samples_total')::bigint
  AND t.nr_completed_samples=(c.sampler_checkpoint->>'completed_samples')::bigint
  AND (SELECT COALESCE(sum(b.batch_size),0) FROM batches b
       WHERE b.run_id=c.run_id AND b.task_id=c.task_id
         AND b.id>COALESCE((c.sampler_checkpoint->'queue'->>'last_completed_batch_id')::bigint,0))
      =t.nr_produced_samples-t.nr_completed_samples;
