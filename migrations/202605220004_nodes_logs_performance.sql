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
