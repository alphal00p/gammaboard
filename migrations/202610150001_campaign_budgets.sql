-- Durable child allowances include completed, queued and locally buffered draws.
-- Only the campaign controller may increase them, under its task row lock.
CREATE TABLE campaign_sample_budgets (
    child_run_id INT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    campaign_task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    granted_samples BIGINT NOT NULL DEFAULT 0 CHECK (granted_samples >= 0)
);
CREATE INDEX idx_campaign_sample_budgets_task ON campaign_sample_budgets(campaign_task_id);
