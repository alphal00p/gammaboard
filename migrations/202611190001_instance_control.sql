-- Scheduler-independent control and export requests. One logical database is one instance.
CREATE TABLE gammaboard_instance_control (
    singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK (singleton),
    owner_id TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    heartbeat_at TIMESTAMPTZ,
    draining BOOLEAN NOT NULL DEFAULT false,
    status JSONB NOT NULL DEFAULT '{}',
    shutdown_id TEXT,
    shutdown_request JSONB,
    shutdown_result JSONB,
    cleanup_required BOOLEAN NOT NULL DEFAULT false,
    cleanup_acknowledged BOOLEAN NOT NULL DEFAULT false
);
INSERT INTO gammaboard_instance_control(singleton) VALUES(true);
CREATE TABLE gammaboard_run_exports (
    id TEXT PRIMARY KEY,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT false,
    error TEXT,
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX gammaboard_pending_export_path ON gammaboard_run_exports(path) WHERE NOT completed;
CREATE TABLE gammaboard_run_starts (
    run_id INTEGER PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- A backend acknowledges scheduler cleanup independently of application worker leases.
ALTER TABLE external_worker_launches ADD COLUMN cleanup_confirmed BOOLEAN NOT NULL DEFAULT false;
