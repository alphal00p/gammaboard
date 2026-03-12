-- Control-plane tables for node registration and desired/current role state.

CREATE TABLE IF NOT EXISTS nodes (
    node_id TEXT PRIMARY KEY,
    desired_run_id INT REFERENCES runs(id) ON DELETE SET NULL,
    desired_role TEXT CHECK (desired_role IN ('evaluator', 'sampler_aggregator')),
    current_run_id INT REFERENCES runs(id) ON DELETE SET NULL,
    current_role TEXT CHECK (current_role IN ('evaluator', 'sampler_aggregator')),
    last_seen TIMESTAMPTZ,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    shutdown_requested_at TIMESTAMPTZ,
    CONSTRAINT nodes_desired_assignment_pair_check CHECK (
        (desired_run_id IS NULL AND desired_role IS NULL)
        OR (desired_run_id IS NOT NULL AND desired_role IS NOT NULL)
    ),
    CONSTRAINT nodes_current_assignment_pair_check CHECK (
        (current_run_id IS NULL AND current_role IS NULL)
        OR (current_run_id IS NOT NULL AND current_role IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_nodes_last_seen
    ON nodes(last_seen DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_desired_sampler_run
    ON nodes(desired_run_id)
    WHERE desired_role = 'sampler_aggregator';

CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_current_sampler_run
    ON nodes(current_run_id)
    WHERE current_role = 'sampler_aggregator';
