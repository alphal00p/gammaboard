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
