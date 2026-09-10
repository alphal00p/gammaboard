ALTER TABLE nodes ADD COLUMN resume_requested BOOLEAN NOT NULL DEFAULT FALSE;
-- The normalized one-worker group retains scheduler settings, not inferred capabilities.
ALTER TABLE nodes ADD COLUMN launch_group JSONB;
ALTER TABLE nodes ADD COLUMN launch_request_id BIGINT REFERENCES node_launch_requests(id);
