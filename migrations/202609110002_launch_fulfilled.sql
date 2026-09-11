ALTER TABLE node_launch_requests DROP CONSTRAINT node_launch_requests_state_check;
UPDATE node_launch_requests SET state = 'fulfilled' WHERE state = 'running';
ALTER TABLE node_launch_requests ADD CONSTRAINT node_launch_requests_state_check
    CHECK (state IN ('pending', 'starting', 'fulfilled', 'failed', 'canceled'));
