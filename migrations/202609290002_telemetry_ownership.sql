-- Register ownership once per worker/run. Repeated telemetry inserts then lock
-- only their own immutable owner, avoiding fleet-wide foreign-key MultiXacts.
CREATE TABLE run_telemetry_workers (
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES nodes(name) ON DELETE CASCADE,
    PRIMARY KEY (run_id, worker_id)
);
INSERT INTO run_telemetry_workers
    SELECT run_id, worker_id FROM evaluator_performance_history
    UNION SELECT run_id, worker_id FROM evaluator_performance_latest
    UNION SELECT run_id, worker_id FROM sampler_aggregator_performance_history
    UNION SELECT run_id, worker_id FROM sampler_aggregator_performance_latest;

ALTER TABLE evaluator_performance_history
    DROP CONSTRAINT evaluator_performance_history_run_id_fkey,
    DROP CONSTRAINT evaluator_performance_history_worker_id_fkey,
    ADD FOREIGN KEY (run_id, worker_id) REFERENCES run_telemetry_workers ON DELETE CASCADE;
ALTER TABLE evaluator_performance_latest
    DROP CONSTRAINT evaluator_performance_latest_run_id_fkey,
    DROP CONSTRAINT evaluator_performance_latest_worker_id_fkey,
    ADD FOREIGN KEY (run_id, worker_id) REFERENCES run_telemetry_workers ON DELETE CASCADE;
ALTER TABLE sampler_aggregator_performance_history
    DROP CONSTRAINT sampler_aggregator_performance_history_run_id_fkey,
    DROP CONSTRAINT sampler_aggregator_performance_history_worker_id_fkey,
    ADD FOREIGN KEY (run_id, worker_id) REFERENCES run_telemetry_workers ON DELETE CASCADE;
ALTER TABLE sampler_aggregator_performance_latest
    DROP CONSTRAINT sampler_aggregator_performance_latest_run_id_fkey,
    DROP CONSTRAINT sampler_aggregator_performance_latest_worker_id_fkey,
    ADD FOREIGN KEY (run_id, worker_id) REFERENCES run_telemetry_workers ON DELETE CASCADE;
