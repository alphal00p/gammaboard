use sqlx::PgConnection;

// Idle workers reserve control + evaluator capacity. Sampler assignment reserves
// the extra role connections under the same lock used by worker launch admission.
const CONNECTIONS_PER_WORKER: i64 = 2 + crate::runners::MAX_EVALUATOR_DB_CONNECTIONS as i64;
const EXTRA_SAMPLER_CONNECTIONS: i64 = crate::runners::MAX_SAMPLER_DB_CONNECTIONS as i64
    - crate::runners::MAX_EVALUATOR_DB_CONNECTIONS as i64;
const CONNECTION_HEADROOM: i64 = 16;

fn capacity(max_connections: i64, reserved_connections: i64, samplers: i64) -> i64 {
    (max_connections
        - reserved_connections
        - CONNECTION_HEADROOM
        - samplers * EXTRA_SAMPLER_CONNECTIONS)
        .max(0)
        / CONNECTIONS_PER_WORKER
}

pub(super) async fn lock(connection: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(71809241)")
        .execute(connection)
        .await?;
    Ok(())
}

/// Called while holding the worker-launch advisory transaction lock, so two
/// launch requests cannot both spend the same remaining connection budget.
pub(super) async fn check_worker_connection_budget(
    connection: &mut PgConnection,
    additional_workers: i64,
) -> Result<(), sqlx::Error> {
    let (max_connections, reserved_connections): (i32, i32) = sqlx::query_as(
        "SELECT current_setting('max_connections')::int,
                current_setting('superuser_reserved_connections')::int +
                COALESCE(current_setting('reserved_connections', true)::int, 0)",
    )
    .fetch_one(&mut *connection)
    .await?;
    // A launch reservation lasts until that worker first connects, not until
    // every worker in the group is simultaneously alive. Expired workers that
    // already connected are history, even if an old launcher left 'starting'.
    // Compare against this request's creation time so a resumed worker's old
    // heartbeat cannot discharge its replacement's reservation.
    let (live, reserved, samplers): (i64, i64, i64) = sqlx::query_as(
        "WITH workers AS (
            SELECT n.lease_expires_at > now() AS live,
                   n.lease_expires_at <= now() AND EXISTS (
                       SELECT 1 FROM node_launch_requests r
                        WHERE r.id=n.launch_request_id AND r.state IN ('pending','starting')
                          AND (n.last_seen IS NULL OR n.last_seen < r.created_at)) AS reserved,
                   'sampler_aggregator' IN (n.pool_role,n.desired_role,n.active_role) AS sampler
            FROM nodes n
         ) SELECT count(*) FILTER (WHERE live), count(*) FILTER (WHERE reserved),
                  count(*) FILTER (WHERE (live OR reserved) AND sampler)
           FROM workers",
    )
    .fetch_one(&mut *connection)
    .await?;
    let pg_reserved = i64::from(reserved_connections);
    let allowed = capacity(i64::from(max_connections), pg_reserved, samplers);
    if live
        .saturating_add(reserved)
        .saturating_add(additional_workers)
        > allowed
    {
        return Err(sqlx::Error::Protocol(format!(
            "worker connection budget exceeded: {live} live workers + {reserved} awaiting registration + \
             {additional_workers} requested exceeds capacity {allowed} at \
             max_connections={max_connections} ({CONNECTIONS_PER_WORKER} connections per \
             worker plus {EXTRA_SAMPLER_CONNECTIONS} for each of {samplers} sampler workers, \
             {CONNECTION_HEADROOM} for server/operators, {pg_reserved} PostgreSQL reserved). \
             Stop unused workers, request fewer workers, or increase PostgreSQL \
             max_connections and restart the database."
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_leaves_room_for_both_worker_pools_and_administration() {
        assert_eq!(capacity(128, 3, 0), 27);
        assert_eq!(capacity(256, 3, 0), 59);
        assert_eq!(capacity(16, 3, 0), 0);
        assert_eq!(capacity(128, 3, 1), 26);
        assert_eq!(capacity(128, 3, 3), 24);
    }
}
