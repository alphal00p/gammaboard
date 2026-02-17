-- Control-plane tables for component registration and run assignments.

CREATE TABLE IF NOT EXISTS component_instances (
    instance_id TEXT PRIMARY KEY,
    node_id TEXT,
    role TEXT NOT NULL CHECK (role IN ('evaluator', 'sampler_aggregator')),
    implementation TEXT NOT NULL,
    version TEXT NOT NULL,
    node_specs JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'draining', 'inactive')),
    last_seen TIMESTAMPTZ,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_component_instances_role_impl
    ON component_instances(role, implementation, version, status);

CREATE INDEX IF NOT EXISTS idx_component_instances_last_seen
    ON component_instances(last_seen DESC);

-- Exactly one sampler-aggregator lease per run.
CREATE TABLE IF NOT EXISTS run_sampler_aggregator_leases (
    run_id INT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    instance_id TEXT NOT NULL REFERENCES component_instances(instance_id) ON DELETE CASCADE,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_sa_leases_expires
    ON run_sampler_aggregator_leases(lease_expires_at);

-- One-to-many evaluator assignments per run.
CREATE TABLE IF NOT EXISTS run_evaluator_assignments (
    run_id INT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    instance_id TEXT NOT NULL REFERENCES component_instances(instance_id) ON DELETE CASCADE,
    active BOOLEAN NOT NULL DEFAULT true,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, instance_id)
);

CREATE INDEX IF NOT EXISTS idx_run_evaluator_assignments_active
    ON run_evaluator_assignments(run_id, active, assigned_at);
