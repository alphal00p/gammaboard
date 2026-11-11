-- A request may still be starting because a different member never connected.
-- Only reserve this worker until it registers for this particular launch.
CREATE FUNCTION worker_launch_pending(worker_name TEXT, launch_id BIGINT, seen_at TIMESTAMPTZ)
RETURNS BOOLEAN LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM node_launch_requests r
        WHERE r.id = launch_id AND r.state IN ('pending', 'starting')
          AND (seen_at IS NULL OR seen_at < r.created_at)
          AND NOT EXISTS (
              SELECT 1 FROM external_worker_launches w
              WHERE w.request_id = r.id AND w.node_name = worker_name
                AND w.state IN ('failed', 'canceled')
          )
    )
$$;
