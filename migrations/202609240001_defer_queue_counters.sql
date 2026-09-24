-- Keep the shared per-run counter unlocked while large input/result payloads
-- are written. Counters still commit atomically with their batch changes.
DROP TRIGGER batches_queue_counter_trigger ON batches;
CREATE CONSTRAINT TRIGGER batches_queue_counter_trigger
AFTER INSERT OR DELETE OR UPDATE OF status ON batches
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW EXECUTE FUNCTION sync_run_batch_queue_counters();
