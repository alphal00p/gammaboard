
-- Source: migrations/202605220001_runs.sql
-- Runs are the top-level integration records.
CREATE TABLE IF NOT EXISTS runs (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    started_at TIMESTAMPTZ DEFAULT now(),
    completed_at TIMESTAMPTZ,
    nr_produced_samples BIGINT NOT NULL DEFAULT 0,
    nr_completed_samples BIGINT NOT NULL DEFAULT 0,
    integration_params JSONB,
    target JSONB,
    point_spec JSONB NOT NULL,
    current_observable JSONB,
    batches_completed INT DEFAULT 0,
    exposed_info JSONB NOT NULL DEFAULT '{}'::jsonb,
    run_toml TEXT,
    parent_run_id INT REFERENCES runs(id) ON DELETE CASCADE,
    parent_task_id BIGINT,
    spawn_kind TEXT,
    spawn_label TEXT,
    sampler_runner_uptime_ms DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    CONSTRAINT runs_sample_progress_check CHECK (
        nr_produced_samples >= 0
        AND nr_completed_samples >= 0
        AND nr_completed_samples <= nr_produced_samples
    ),
    CONSTRAINT runs_sampler_runner_uptime_check CHECK (
        sampler_runner_uptime_ms >= 0.0
    )
);

CREATE INDEX IF NOT EXISTS idx_runs_started_at
    ON runs(started_at DESC);

CREATE INDEX IF NOT EXISTS idx_runs_parent_run_id
    ON runs(parent_run_id, started_at DESC)
    WHERE parent_run_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_runs_parent_task_id
    ON runs(parent_task_id, started_at DESC)
    WHERE parent_task_id IS NOT NULL;

-- Source: migrations/202605220002_tasks_batches_outputs.sql
-- Run task queue and sample work queue.
CREATE TABLE IF NOT EXISTS run_tasks (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    sequence_nr INT NOT NULL,
    task JSONB NOT NULL,
    task_toml TEXT NOT NULL,
    spawned_from_snapshot_id BIGINT,
    state TEXT NOT NULL DEFAULT 'pending',
    nr_produced_samples BIGINT NOT NULL DEFAULT 0,
    nr_completed_samples BIGINT NOT NULL DEFAULT 0,
    measurement_output JSONB,
    controller_output JSONB,
    failure_reason TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    failed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT run_tasks_state_check CHECK (
        state IN ('pending', 'active', 'completed', 'failed')
    ),
    CONSTRAINT run_tasks_progress_check CHECK (
        nr_produced_samples >= 0
        AND nr_completed_samples >= 0
        AND nr_completed_samples <= nr_produced_samples
    ),
    CONSTRAINT run_tasks_stop_condition_shape CHECK (
        CASE
            WHEN task->>'kind' IN ('sample', 'integration_campaign') THEN
                jsonb_typeof(task->'stop_condition') = 'object'
                AND NOT (task ? 'nr_samples')
            ELSE
                NOT (task ? 'stop_condition')
                AND NOT (task ? 'nr_samples')
        END
    ),
    CONSTRAINT run_tasks_sequence_unique UNIQUE (run_id, sequence_nr),
    CONSTRAINT run_tasks_name_unique UNIQUE (run_id, name)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_run_tasks_active_one_per_run
    ON run_tasks(run_id)
    WHERE state = 'active';

CREATE INDEX IF NOT EXISTS idx_run_tasks_run_sequence
    ON run_tasks(run_id, sequence_nr, id);

CREATE INDEX IF NOT EXISTS idx_run_tasks_run_name
    ON run_tasks(run_id, name);

CREATE TABLE IF NOT EXISTS batches (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    requires_training_values BOOLEAN NOT NULL DEFAULT FALSE,
    batch_size INT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    claimed_by_node_name TEXT,
    claimed_by_node_uuid TEXT,
    claimed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT now(),
    retry_count INT DEFAULT 0,
    last_error TEXT
);

CREATE TABLE IF NOT EXISTS batch_inputs (
    batch_id BIGINT PRIMARY KEY REFERENCES batches(id) ON DELETE CASCADE,
    latent_batch BYTEA NOT NULL
);

CREATE TABLE IF NOT EXISTS batch_results (
    batch_id BIGINT PRIMARY KEY REFERENCES batches(id) ON DELETE CASCADE,
    "values" BYTEA,
    batch_observable JSONB NOT NULL,
    total_eval_time_ms DOUBLE PRECISION,
    completed_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_batches_status_runid
    ON batches(run_id, status)
    WHERE status IN ('pending', 'claimed');

CREATE INDEX IF NOT EXISTS idx_batches_pending_run_created
    ON batches(run_id, created_at, id)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS idx_batches_task_created
    ON batches(task_id, created_at);

CREATE INDEX IF NOT EXISTS idx_batches_claimed
    ON batches(claimed_at)
    WHERE status = 'claimed';

CREATE INDEX IF NOT EXISTS idx_batches_completed
    ON batches(run_id, completed_at)
    WHERE status = 'completed';

CREATE INDEX IF NOT EXISTS idx_batches_completed_run_id
    ON batches(run_id, id)
    WHERE status = 'completed';

CREATE INDEX IF NOT EXISTS idx_batches_run_id_id
    ON batches(run_id, id);

CREATE INDEX IF NOT EXISTS idx_batch_results_completed_at
    ON batch_results(completed_at);

-- Compact task-local panel/output history.
CREATE TABLE IF NOT EXISTS persisted_observable_snapshots (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT NOT NULL,
    persisted_observable JSONB NOT NULL,
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_persisted_observable_snapshots_run_task_id
    ON persisted_observable_snapshots(run_id, task_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_persisted_observable_snapshots_created
    ON persisted_observable_snapshots(created_at);

CREATE INDEX IF NOT EXISTS idx_persisted_observable_snapshots_run_task_id_id_desc
    ON persisted_observable_snapshots(run_id, task_id, id DESC);

-- Source: migrations/202605220003_stage_snapshots_checkpoints.sql
-- Branchable stage state and sampler handoff checkpoints.
CREATE TABLE IF NOT EXISTS run_stage_snapshots (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT REFERENCES run_tasks(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    sequence_nr INT,
    queue_empty BOOLEAN NOT NULL,
    sampler_snapshot JSONB,
    observable_state JSONB,
    evaluator JSONB,
    sampler_aggregator JSONB,
    batch_transforms JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_run_stage_snapshots_run_created
    ON run_stage_snapshots(run_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_run_stage_snapshots_run_sequence
    ON run_stage_snapshots(run_id, sequence_nr, created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS uq_run_stage_snapshots_root
    ON run_stage_snapshots(run_id)
    WHERE queue_empty = TRUE
      AND task_id IS NULL
      AND sequence_nr = 0;

ALTER TABLE run_tasks
    ADD CONSTRAINT run_tasks_spawned_from_snapshot_fkey
    FOREIGN KEY (spawned_from_snapshot_id) REFERENCES run_stage_snapshots(id) ON DELETE SET NULL;

CREATE TABLE IF NOT EXISTS run_sampler_checkpoints (
    run_id INT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    task_id BIGINT NOT NULL REFERENCES run_tasks(id) ON DELETE CASCADE,
    sampler_checkpoint JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Source: migrations/202605220004_nodes_logs_performance.sql
-- Node control plane, runtime logs, and worker performance history.
CREATE TABLE IF NOT EXISTS nodes (
    name TEXT PRIMARY KEY,
    uuid TEXT NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    desired_run_id INT REFERENCES runs(id) ON DELETE SET NULL,
    desired_role TEXT CHECK (desired_role IN ('evaluator', 'sampler_aggregator')),
    active_run_id INT REFERENCES runs(id) ON DELETE SET NULL,
    active_role TEXT CHECK (active_role IN ('evaluator', 'sampler_aggregator')),
    capabilities JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_seen TIMESTAMPTZ,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    shutdown_requested_at TIMESTAMPTZ,
    CONSTRAINT nodes_desired_assignment_pair_check CHECK (
        (desired_run_id IS NULL AND desired_role IS NULL)
        OR (desired_run_id IS NOT NULL AND desired_role IS NOT NULL)
    ),
    CONSTRAINT nodes_current_assignment_pair_check CHECK (
        (active_run_id IS NULL AND active_role IS NULL)
        OR (active_run_id IS NOT NULL AND active_role IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_nodes_last_seen
    ON nodes(last_seen DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_desired_sampler_run
    ON nodes(desired_run_id)
    WHERE desired_role = 'sampler_aggregator';

CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_current_sampler_run
    ON nodes(active_run_id)
    WHERE active_role = 'sampler_aggregator';

CREATE INDEX IF NOT EXISTS idx_nodes_active_evaluator_run
    ON nodes(active_run_id)
    WHERE active_role = 'evaluator';

CREATE TABLE IF NOT EXISTS runtime_logs (
    id BIGSERIAL PRIMARY KEY,
    ts TIMESTAMPTZ NOT NULL DEFAULT now(),
    source TEXT NOT NULL,
    run_id INT,
    node_uuid TEXT,
    node_name TEXT,
    level TEXT NOT NULL,
    target TEXT NOT NULL,
    message TEXT NOT NULL,
    fields JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_run_id
    ON runtime_logs(source, run_id, id DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_node_name
    ON runtime_logs(source, node_name, id DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_node_uuid
    ON runtime_logs(source, node_uuid, id DESC);

CREATE INDEX IF NOT EXISTS idx_runtime_logs_source_level
    ON runtime_logs(source, level, id DESC);

CREATE TABLE IF NOT EXISTS evaluator_performance_history (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES nodes(name) ON DELETE CASCADE,
    metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    rss_bytes BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_eval_perf_history_run_worker_time
    ON evaluator_performance_history(run_id, worker_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_eval_perf_history_run_time
    ON evaluator_performance_history(run_id, created_at DESC);

CREATE TABLE IF NOT EXISTS evaluator_performance_latest (
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES nodes(name) ON DELETE CASCADE,
    id BIGINT NOT NULL,
    metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    rss_bytes BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, worker_id)
);

CREATE TABLE IF NOT EXISTS sampler_aggregator_performance_history (
    id BIGSERIAL PRIMARY KEY,
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES nodes(name) ON DELETE CASCADE,
    metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    runtime_metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    engine_diagnostics JSONB NOT NULL DEFAULT '{}'::jsonb,
    rss_bytes BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sampler_perf_history_run_worker_time
    ON sampler_aggregator_performance_history(run_id, worker_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_sampler_perf_history_run_time
    ON sampler_aggregator_performance_history(run_id, created_at DESC);

CREATE TABLE IF NOT EXISTS sampler_aggregator_performance_latest (
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES nodes(name) ON DELETE CASCADE,
    id BIGINT NOT NULL,
    metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    runtime_metrics JSONB NOT NULL DEFAULT '{}'::jsonb,
    engine_diagnostics JSONB NOT NULL DEFAULT '{}'::jsonb,
    rss_bytes BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, worker_id)
);

-- Source: migrations/202605220005_node_launch_requests.sql
-- Persisted requests for external node launch backends.
CREATE TABLE IF NOT EXISTS node_launch_requests (
    id BIGSERIAL PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    state TEXT NOT NULL CHECK (state IN ('pending', 'starting', 'running', 'failed', 'canceled')),
    backend TEXT NOT NULL,
    requested_count INT NOT NULL CHECK (requested_count > 0),
    started_count INT NOT NULL DEFAULT 0 CHECK (started_count >= 0),
    name_prefix TEXT,
    args JSONB NOT NULL DEFAULT '{}'::jsonb,
    result JSONB NOT NULL DEFAULT '{}'::jsonb,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_node_launch_requests_state_created
    ON node_launch_requests(state, created_at);

CREATE INDEX IF NOT EXISTS idx_node_launch_requests_created
    ON node_launch_requests(created_at DESC);

-- Source: migrations/202605220006_run_provenance.sql
ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS provenance JSONB NOT NULL DEFAULT '{}'::jsonb;

-- Source: migrations/202608180001_batch_queue_counters.sql
-- Keep hot queue state reads independent of retained failed-batch history.
CREATE TABLE IF NOT EXISTS run_batch_queue_counters (
    run_id INT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    total_batches BIGINT NOT NULL DEFAULT 0 CHECK (total_batches >= 0),
    total_samples BIGINT NOT NULL DEFAULT 0 CHECK (total_samples >= 0),
    pending_batches BIGINT NOT NULL DEFAULT 0 CHECK (pending_batches >= 0),
    claimed_batches BIGINT NOT NULL DEFAULT 0 CHECK (claimed_batches >= 0),
    completed_batches BIGINT NOT NULL DEFAULT 0 CHECK (completed_batches >= 0),
    failed_batches BIGINT NOT NULL DEFAULT 0 CHECK (failed_batches >= 0)
);

INSERT INTO run_batch_queue_counters (
    run_id, total_batches, total_samples, pending_batches, claimed_batches, completed_batches, failed_batches
)
SELECT
    run_id,
    COUNT(*),
    COALESCE(SUM(batch_size), 0),
    COUNT(*) FILTER (WHERE status = 'pending'),
    COUNT(*) FILTER (WHERE status = 'claimed'),
    COUNT(*) FILTER (WHERE status = 'completed'),
    COUNT(*) FILTER (WHERE status = 'failed')
FROM batches
GROUP BY run_id
ON CONFLICT (run_id) DO UPDATE SET
    total_batches = EXCLUDED.total_batches,
    total_samples = EXCLUDED.total_samples,
    pending_batches = EXCLUDED.pending_batches,
    claimed_batches = EXCLUDED.claimed_batches,
    completed_batches = EXCLUDED.completed_batches,
    failed_batches = EXCLUDED.failed_batches;

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
    SELECT target_run_id
    FROM runs
    WHERE id = target_run_id
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

DROP TRIGGER IF EXISTS batches_queue_counter_trigger ON batches;
CREATE TRIGGER batches_queue_counter_trigger
AFTER INSERT OR DELETE OR UPDATE OF status ON batches
FOR EACH ROW EXECUTE FUNCTION sync_run_batch_queue_counters();

CREATE INDEX IF NOT EXISTS idx_runs_controller_children
    ON runs(parent_run_id, parent_task_id, spawn_kind, started_at DESC, id DESC)
    WHERE parent_run_id IS NOT NULL;
