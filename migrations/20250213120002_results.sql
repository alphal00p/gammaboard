-- Aggregated results: periodic run-level snapshots of the aggregated observable.
CREATE TABLE IF NOT EXISTS aggregated_results (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    aggregated_observable JSONB NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_aggregated_results_run_id
    ON aggregated_results(run_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_aggregated_results_created
    ON aggregated_results(created_at);
