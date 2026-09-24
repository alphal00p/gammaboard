use sqlx::PgConnection;

// Two control connections (one may hold the controller lock), plus two role
// connections. Budget idle workers as assignable, rather than allowing a later
// assignment to exhaust PostgreSQL. Leave space for the server and operators.
const CONNECTIONS_PER_WORKER: i64 = 4;
const CONNECTION_HEADROOM: i64 = 16;

fn capacity(max_connections: i64, reserved_connections: i64) -> i64 {
    (max_connections - reserved_connections - CONNECTION_HEADROOM).max(0) / CONNECTIONS_PER_WORKER
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
    let (live, reserved): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE n.lease_expires_at > now()),
                count(*) FILTER (WHERE n.lease_expires_at <= now()
                    AND EXISTS (SELECT 1 FROM node_launch_requests r
                        WHERE r.id=n.launch_request_id AND r.state IN ('pending','starting')
                          AND (n.last_seen IS NULL OR n.last_seen < r.created_at)))
         FROM nodes n",
    )
    .fetch_one(&mut *connection)
    .await?;
    let pg_reserved = i64::from(reserved_connections);
    let allowed = capacity(i64::from(max_connections), pg_reserved);
    if live
        .saturating_add(reserved)
        .saturating_add(additional_workers)
        > allowed
    {
        return Err(sqlx::Error::Protocol(format!(
            "worker connection budget exceeded: {live} live workers + {reserved} awaiting registration + \
             {additional_workers} requested exceeds capacity {allowed} at \
             max_connections={max_connections} ({CONNECTIONS_PER_WORKER} connections per \
             worker, {CONNECTION_HEADROOM} for server/operators, {pg_reserved} PostgreSQL reserved). \
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
        assert_eq!(capacity(128, 3), 27);
        assert_eq!(capacity(256, 3), 59);
        assert_eq!(capacity(16, 3), 0);
    }
}
