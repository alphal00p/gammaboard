CREATE OR REPLACE FUNCTION sync_run_batch_queue_counters()
RETURNS TRIGGER AS $$
DECLARE
    old_pending BIGINT := 0;
    old_claimed BIGINT := 0;
    old_completed BIGINT := 0;
    old_failed BIGINT := 0;
    old_samples BIGINT := 0;
    new_pending BIGINT := 0;
    new_claimed BIGINT := 0;
    new_completed BIGINT := 0;
    new_failed BIGINT := 0;
    new_samples BIGINT := 0;
    target_run_id INT;
BEGIN
    IF TG_OP IN ('UPDATE', 'DELETE') THEN
        target_run_id := OLD.run_id;
        old_pending := CASE WHEN OLD.status = 'pending' THEN 1 ELSE 0 END;
        old_claimed := CASE WHEN OLD.status = 'claimed' THEN 1 ELSE 0 END;
        old_completed := CASE WHEN OLD.status = 'completed' THEN 1 ELSE 0 END;
        old_failed := CASE WHEN OLD.status = 'failed' THEN 1 ELSE 0 END;
        old_samples := OLD.batch_size;
    END IF;
    IF TG_OP IN ('UPDATE', 'INSERT') THEN
        target_run_id := NEW.run_id;
        new_pending := CASE WHEN NEW.status = 'pending' THEN 1 ELSE 0 END;
        new_claimed := CASE WHEN NEW.status = 'claimed' THEN 1 ELSE 0 END;
        new_completed := CASE WHEN NEW.status = 'completed' THEN 1 ELSE 0 END;
        new_failed := CASE WHEN NEW.status = 'failed' THEN 1 ELSE 0 END;
        new_samples := NEW.batch_size;
    END IF;

    INSERT INTO run_batch_queue_counters (run_id)
    VALUES (target_run_id)
    ON CONFLICT (run_id) DO NOTHING;

    UPDATE run_batch_queue_counters
    SET
        total_batches = total_batches + CASE WHEN TG_OP = 'INSERT' THEN 1 WHEN TG_OP = 'DELETE' THEN -1 ELSE 0 END,
        total_samples = total_samples + new_samples - old_samples,
        pending_batches = pending_batches + new_pending - old_pending,
        claimed_batches = claimed_batches + new_claimed - old_claimed,
        completed_batches = completed_batches + new_completed - old_completed,
        failed_batches = failed_batches + new_failed - old_failed
    WHERE run_id = target_run_id;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
