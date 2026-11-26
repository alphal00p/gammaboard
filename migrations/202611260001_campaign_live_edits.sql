-- Requests remain in force until the controller has reconciled the drained work.
CREATE TABLE run_drain_requests (
    id BIGSERIAL UNIQUE NOT NULL,
    run_id INT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    acknowledged BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE campaign_child_edits (
    id BIGSERIAL PRIMARY KEY,
    campaign_task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    run_id INT NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('delete','adopt')),
    payload JSONB NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','completed','failed')),
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX campaign_child_edit_pending ON campaign_child_edits(run_id) WHERE state='pending';
-- Retain expenditure after a child's records are deleted.
CREATE TABLE campaign_removed_work (
    campaign_task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    child_run_id INT NOT NULL,
    label TEXT NOT NULL,
    produced_samples BIGINT NOT NULL,
    completed_samples BIGINT NOT NULL,
    cpu_seconds DOUBLE PRECISION NOT NULL,
    PRIMARY KEY(campaign_task_id,child_run_id)
);
CREATE TABLE campaign_edit_receipts (
    campaign_task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL,
    request JSONB NOT NULL,
    response JSONB NOT NULL,
    PRIMARY KEY(campaign_task_id,request_id)
);
