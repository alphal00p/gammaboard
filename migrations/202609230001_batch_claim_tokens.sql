-- A claim token fences stale submissions, including reassignment to the same node.
-- Keeping it on completed/requeued rows also makes acknowledgement retries safe.
ALTER TABLE batches ADD COLUMN claim_token TEXT;
CREATE UNIQUE INDEX idx_batches_claim_token ON batches(claim_token)
    WHERE claim_token IS NOT NULL;
CREATE INDEX idx_batches_task_id_id ON batches(task_id, id);
