-- Control-plane tables for worker registration and run assignments.

CREATE TABLE IF NOT EXISTS workers (
    worker_id TEXT PRIMARY KEY,
    node_id TEXT,
    role TEXT NOT NULL CHECK (role IN ('evaluator', 'sampler_aggregator')),
    implementation TEXT NOT NULL,
    version TEXT NOT NULL,
    node_specs JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'draining', 'inactive')),
    desired_run_id INT REFERENCES runs(id) ON DELETE SET NULL,
    desired_updated_at TIMESTAMPTZ, --this is set by the control plane
    last_seen TIMESTAMPTZ,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    shutdown_requested_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS idx_workers_role_impl
    ON workers(role, implementation, version, status);

CREATE INDEX IF NOT EXISTS idx_workers_last_seen
    ON workers(last_seen DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_workers_node_role
    ON workers(node_id, role)
    WHERE node_id IS NOT NULL;

-- Exactly one sampler-aggregator lease per run.
CREATE TABLE IF NOT EXISTS run_sampler_aggregator_leases (
    run_id INT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES workers(worker_id) ON DELETE CASCADE,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sa_leases_expires
    ON run_sampler_aggregator_leases(lease_expires_at);

-- One-to-many evaluator assignments per run. represents actual state
CREATE TABLE IF NOT EXISTS run_evaluator_assignments (
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    worker_id TEXT NOT NULL REFERENCES workers(worker_id) ON DELETE CASCADE,
    active BOOLEAN NOT NULL DEFAULT true,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, worker_id)
);

CREATE INDEX IF NOT EXISTS idx_run_evaluator_assignments_active
    ON run_evaluator_assignments(run_id, active, assigned_at);
