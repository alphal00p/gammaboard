-- Keep only durable progress, and let the queue own the batch size. Do not
-- manufacture production boundaries for checkpoints that cannot be recovered.
UPDATE run_sampler_checkpoints
SET sampler_checkpoint = jsonb_set(
    jsonb_set(
        sampler_checkpoint,
        '{runtime_state}',
        (sampler_checkpoint->'runtime_state') - ARRAY[
            'completed_samples_per_second', 'eta_seconds', 'sampler_tick_busy_ratio',
            'initial_round_trip_snapshot_pending', 'pending_persisted_completed_batches',
            'batch_size_current'
        ]
    ),
    '{batches_completed}',
    COALESCE(NULLIF(sampler_checkpoint->'batches_completed', 'null'::jsonb), '0'::jsonb)
)
WHERE sampler_checkpoint ? 'runtime_state';
