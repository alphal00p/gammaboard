use crate::core::{NodeCapabilities, WorkerRole};
use serde_json::Value as JsonValue;
use sqlx::{PgPool, postgres::PgQueryResult};

const CLEAR_DESIRED_ASSIGNMENT_SET: &str = r#"
    desired_run_id = NULL,
    desired_role = NULL,
    updated_at = now()
"#;

const CLEAR_CURRENT_ASSIGNMENT_SET: &str = r#"
    active_run_id = NULL,
    active_role = NULL,
    updated_at = now()
"#;

pub(crate) struct DesiredAssignmentRaw {
    pub node_name: String,
    pub role: String,
    pub run_id: i32,
}

pub(crate) struct NodeRaw {
    pub name: String,
    pub uuid: String,
    pub capabilities: JsonValue,
    pub desired_role: Option<String>,
    pub desired_run_id: Option<i32>,
    pub desired_run_name: Option<String>,
    pub current_role: Option<String>,
    pub current_run_id: Option<i32>,
    pub current_run_name: Option<String>,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
}

pub(crate) type NodeLaunchRequestRaw = (
    i64,
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Utc>,
    String,
    String,
    i32,
    i32,
    Option<String>,
    JsonValue,
    JsonValue,
    Option<String>,
);

fn stale_node_uuid_error(node_uuid: &str) -> sqlx::Error {
    sqlx::Error::Protocol(format!("node uuid '{node_uuid}' is no longer live"))
}

fn require_live_uuid(result: PgQueryResult, node_uuid: &str) -> Result<(), sqlx::Error> {
    if result.rows_affected() == 0 {
        Err(stale_node_uuid_error(node_uuid))
    } else {
        Ok(())
    }
}

async fn clear_expired_assignments(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE nodes
        SET
            desired_run_id = NULL,
            desired_role = NULL,
            active_run_id = NULL,
            active_role = NULL,
            updated_at = now()
        WHERE lease_expires_at <= now()
          AND (
            desired_run_id IS NOT NULL
            OR desired_role IS NOT NULL
            OR active_run_id IS NOT NULL
            OR active_role IS NOT NULL
          )
        "#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn desired_assignment_raw(
    (node_name, role, run_id): (String, String, i32),
) -> DesiredAssignmentRaw {
    DesiredAssignmentRaw {
        node_name,
        role,
        run_id,
    }
}

fn node_raw(
    (
        name,
        uuid,
        capabilities,
        desired_role,
        desired_run_id,
        desired_run_name,
        current_role,
        current_run_id,
        current_run_name,
        last_seen,
    ): (
        String,
        String,
        JsonValue,
        Option<String>,
        Option<i32>,
        Option<String>,
        Option<String>,
        Option<i32>,
        Option<String>,
        Option<chrono::DateTime<chrono::Utc>>,
    ),
) -> NodeRaw {
    NodeRaw {
        name,
        uuid,
        capabilities,
        desired_role,
        desired_run_id,
        desired_run_name,
        current_role,
        current_run_id,
        current_run_name,
        last_seen,
    }
}

pub(crate) async fn upsert_desired_assignment(
    pool: &PgPool,
    node_name: &str,
    role: WorkerRole,
    run_id: i32,
) -> Result<bool, sqlx::Error> {
    clear_expired_assignments(pool).await?;
    let result = sqlx::query(
        r#"
        UPDATE nodes
        SET
            desired_run_id = $2,
            desired_role = $3,
            updated_at = now()
        WHERE name = $1
          AND lease_expires_at > now()
        "#,
    )
    .bind(node_name)
    .bind(run_id)
    .bind(role.as_str())
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn announce_node(
    pool: &PgPool,
    node_name: &str,
    node_uuid: &str,
    capabilities: &NodeCapabilities,
) -> Result<(), sqlx::Error> {
    let row = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO nodes (
            name,
            uuid,
            capabilities,
            lease_expires_at,
            last_seen,
            updated_at
        ) VALUES (
            $1,
            $2,
            $3,
            now() + interval '10 seconds',
            now(),
            now()
        )
        ON CONFLICT (name) DO UPDATE
        SET
            uuid = EXCLUDED.uuid,
            capabilities = EXCLUDED.capabilities,
            lease_expires_at = EXCLUDED.lease_expires_at,
            last_seen = EXCLUDED.last_seen,
            updated_at = EXCLUDED.updated_at,
            desired_run_id = CASE
                WHEN nodes.uuid = EXCLUDED.uuid THEN nodes.desired_run_id
                WHEN nodes.lease_expires_at <= now() THEN NULL
                ELSE nodes.desired_run_id
            END,
            desired_role = CASE
                WHEN nodes.uuid = EXCLUDED.uuid THEN nodes.desired_role
                WHEN nodes.lease_expires_at <= now() THEN NULL
                ELSE nodes.desired_role
            END,
            active_run_id = CASE
                WHEN nodes.uuid = EXCLUDED.uuid THEN nodes.active_run_id
                WHEN nodes.lease_expires_at <= now() THEN NULL
                ELSE nodes.active_run_id
            END,
            active_role = CASE
                WHEN nodes.uuid = EXCLUDED.uuid THEN nodes.active_role
                WHEN nodes.lease_expires_at <= now() THEN NULL
                ELSE nodes.active_role
            END,
            shutdown_requested_at = CASE
                WHEN nodes.uuid = EXCLUDED.uuid THEN nodes.shutdown_requested_at
                ELSE NULL
            END
        WHERE nodes.uuid = EXCLUDED.uuid OR nodes.lease_expires_at <= now()
        RETURNING 1
        "#,
    )
    .bind(node_name)
    .bind(node_uuid)
    .bind(
        serde_json::to_value(capabilities)
            .unwrap_or_else(|_| JsonValue::Object(Default::default())),
    )
    .fetch_optional(pool)
    .await?;

    if row.is_some() {
        Ok(())
    } else {
        Err(sqlx::Error::Protocol(format!(
            "node name '{node_name}' is already owned by another live node uuid"
        )))
    }
}

pub(crate) async fn clear_desired_assignment(
    pool: &PgPool,
    node_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        r#"
        UPDATE nodes
        SET
            {set_clause}
        WHERE name = $1
        "#,
        set_clause = CLEAR_DESIRED_ASSIGNMENT_SET
    ))
    .bind(node_name)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn clear_desired_assignments_for_run(
    pool: &PgPool,
    run_id: i32,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(&format!(
        r#"
        UPDATE nodes
        SET
            {set_clause}
        WHERE desired_run_id = $1
        "#,
        set_clause = CLEAR_DESIRED_ASSIGNMENT_SET
    ))
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn clear_desired_assignments_for_run_except_node(
    pool: &PgPool,
    run_id: i32,
    keep_node_name: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(&format!(
        r#"
        UPDATE nodes
        SET
            {set_clause}
        WHERE desired_run_id = $1
          AND name <> $2
        "#,
        set_clause = CLEAR_DESIRED_ASSIGNMENT_SET
    ))
    .bind(run_id)
    .bind(keep_node_name)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn clear_all_desired_assignments(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(&format!(
        r#"
        UPDATE nodes
        SET
            {set_clause}
        "#,
        set_clause = CLEAR_DESIRED_ASSIGNMENT_SET
    ))
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn get_desired_assignment(
    pool: &PgPool,
    node_name: &str,
) -> Result<Option<DesiredAssignmentRaw>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, String, i32)>(
        r#"
        SELECT name, desired_role AS role, desired_run_id AS run_id
        FROM nodes
        WHERE name = $1
          AND desired_run_id IS NOT NULL
        LIMIT 1
        "#,
    )
    .bind(node_name)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(desired_assignment_raw))
}

pub(crate) async fn list_desired_assignments(
    pool: &PgPool,
    node_name: Option<&str>,
) -> Result<Vec<DesiredAssignmentRaw>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, i32)>(
        r#"
        SELECT name, desired_role AS role, desired_run_id AS run_id
        FROM nodes
        WHERE desired_run_id IS NOT NULL
          AND ($1::text IS NULL OR name = $1)
        ORDER BY name ASC
        "#,
    )
    .bind(node_name)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(desired_assignment_raw).collect())
}

pub(crate) async fn list_nodes(
    pool: &PgPool,
    node_name: Option<&str>,
) -> Result<Vec<NodeRaw>, sqlx::Error> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            JsonValue,
            Option<String>,
            Option<i32>,
            Option<String>,
            Option<String>,
            Option<i32>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        r#"
        SELECT
            n.name,
            n.uuid,
            n.capabilities,
            n.desired_role,
            n.desired_run_id,
            dr.name AS desired_run_name,
            n.active_role AS current_role,
            n.active_run_id AS current_run_id,
            cr.name AS current_run_name,
            n.last_seen
        FROM nodes n
        LEFT JOIN runs dr ON dr.id = n.desired_run_id
        LEFT JOIN runs cr ON cr.id = n.active_run_id
        WHERE n.lease_expires_at > now()
          AND ($1::text IS NULL OR n.name = $1)
        ORDER BY n.name ASC
        "#,
    )
    .bind(node_name)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(node_raw).collect())
}

pub(crate) async fn create_node_launch_request(
    pool: &PgPool,
    backend: &str,
    requested_count: i32,
    name_prefix: Option<&str>,
    args: &JsonValue,
) -> Result<NodeLaunchRequestRaw, sqlx::Error> {
    sqlx::query_as::<_, NodeLaunchRequestRaw>(
        r#"
        INSERT INTO node_launch_requests (
            state,
            backend,
            requested_count,
            name_prefix,
            args
        ) VALUES (
            'pending',
            $1,
            $2,
            $3,
            $4
        )
        RETURNING
            id,
            created_at,
            updated_at,
            state,
            backend,
            requested_count,
            started_count,
            name_prefix,
            args,
            result,
            error
        "#,
    )
    .bind(backend)
    .bind(requested_count)
    .bind(name_prefix)
    .bind(args)
    .fetch_one(pool)
    .await
}

pub(crate) async fn list_node_launch_requests(
    pool: &PgPool,
) -> Result<Vec<NodeLaunchRequestRaw>, sqlx::Error> {
    sqlx::query_as::<_, NodeLaunchRequestRaw>(
        r#"
        SELECT
            id,
            created_at,
            updated_at,
            state,
            backend,
            requested_count,
            started_count,
            name_prefix,
            args,
            result,
            error
        FROM node_launch_requests
        ORDER BY created_at DESC
        LIMIT 100
        "#,
    )
    .fetch_all(pool)
    .await
}

pub(crate) async fn claim_external_node_launch_request(
    pool: &PgPool,
) -> Result<Option<NodeLaunchRequestRaw>, sqlx::Error> {
    sqlx::query_as::<_, NodeLaunchRequestRaw>(
        r#"
        WITH next_request AS (
            SELECT id
            FROM node_launch_requests
            WHERE state = 'pending'
              AND backend = 'external'
            ORDER BY created_at, id
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE node_launch_requests request
        SET
            state = 'starting',
            started_count = 0,
            result = '{}'::jsonb,
            error = NULL,
            updated_at = now()
        FROM next_request
        WHERE request.id = next_request.id
        RETURNING
            request.id,
            request.created_at,
            request.updated_at,
            request.state,
            request.backend,
            request.requested_count,
            request.started_count,
            request.name_prefix,
            request.args,
            request.result,
            request.error
        "#,
    )
    .fetch_optional(pool)
    .await
}

pub(crate) async fn reconcile_running_node_launch_requests(
    pool: &PgPool,
) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE node_launch_requests request
        SET
            state = 'running',
            updated_at = now()
        WHERE request.state = 'starting'
          AND request.started_count >= request.requested_count
          AND (
              SELECT COUNT(*)
              FROM jsonb_array_elements(COALESCE(request.result->'workers', '[]'::jsonb)) worker
              JOIN nodes node ON node.name = worker->>'node_name'
              WHERE node.lease_expires_at > now()
          ) >= request.requested_count
        "#,
    )
    .execute(pool)
    .await
}

pub(crate) async fn update_node_launch_request_state(
    pool: &PgPool,
    id: i64,
    state: &str,
    started_count: i32,
    result: &JsonValue,
    error: Option<&str>,
) -> Result<NodeLaunchRequestRaw, sqlx::Error> {
    sqlx::query_as::<_, NodeLaunchRequestRaw>(
        r#"
        UPDATE node_launch_requests
        SET
            state = $2,
            started_count = $3,
            result = $4,
            error = $5,
            updated_at = now()
        WHERE id = $1
        RETURNING
            id,
            created_at,
            updated_at,
            state,
            backend,
            requested_count,
            started_count,
            name_prefix,
            args,
            result,
            error
        "#,
    )
    .bind(id)
    .bind(state)
    .bind(started_count)
    .bind(result)
    .bind(error)
    .fetch_one(pool)
    .await
}

pub(crate) async fn count_active_evaluator_nodes(
    pool: &PgPool,
    run_id: i32,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM nodes
        WHERE lease_expires_at > now()
          AND active_run_id = $1
          AND active_role = 'evaluator'
        "#,
    )
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub(crate) async fn set_current_assignment(
    pool: &PgPool,
    node_uuid: &str,
    role: WorkerRole,
    run_id: i32,
) -> Result<(), sqlx::Error> {
    clear_expired_assignments(pool).await?;
    let result = sqlx::query(
        r#"
        UPDATE nodes
        SET
            active_run_id = $2,
            active_role = $3,
            updated_at = now()
        WHERE uuid = $1
          AND lease_expires_at > now()
        "#,
    )
    .bind(node_uuid)
    .bind(run_id)
    .bind(role.as_str())
    .execute(pool)
    .await?;
    require_live_uuid(result, node_uuid)
}

pub(crate) async fn clear_current_assignment(
    pool: &PgPool,
    node_uuid: &str,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(&format!(
        r#"
        UPDATE nodes
        SET
            {set_clause}
        WHERE uuid = $1
          AND lease_expires_at > now()
        "#,
        set_clause = CLEAR_CURRENT_ASSIGNMENT_SET
    ))
    .bind(node_uuid)
    .execute(pool)
    .await?;
    require_live_uuid(result, node_uuid)
}

pub(crate) async fn request_node_shutdown(
    pool: &PgPool,
    node_name: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        INSERT INTO nodes (
            name,
            uuid,
            lease_expires_at,
            desired_run_id,
            desired_role,
            shutdown_requested_at,
            updated_at
        )
        VALUES
            ($1, '', to_timestamp(0), NULL, NULL, now(), now())
        ON CONFLICT (name) DO UPDATE
        SET
            desired_run_id = NULL,
            desired_role = NULL,
            shutdown_requested_at = now(),
            updated_at = now()
        "#,
    )
    .bind(node_name)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub(crate) async fn request_all_nodes_shutdown(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE nodes
        SET
            desired_run_id = NULL,
            desired_role = NULL,
            shutdown_requested_at = now(),
            updated_at = now()
        WHERE lease_expires_at > now()
        "#,
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub(crate) async fn consume_node_shutdown_request(
    pool: &PgPool,
    node_uuid: &str,
) -> Result<bool, sqlx::Error> {
    let requested = sqlx::query_scalar(
        r#"
        WITH cleared AS (
            UPDATE nodes
            SET
                shutdown_requested_at = NULL,
                updated_at = now()
            WHERE uuid = $1
              AND shutdown_requested_at IS NOT NULL
            RETURNING 1
        )
        SELECT EXISTS(SELECT 1 FROM cleared)
        "#,
    )
    .bind(node_uuid)
    .fetch_one(pool)
    .await?;

    Ok(requested)
}

pub(crate) async fn expire_node_lease(pool: &PgPool, node_uuid: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE nodes
        SET
            lease_expires_at = now(),
            desired_run_id = NULL,
            desired_role = NULL,
            active_run_id = NULL,
            active_role = NULL,
            shutdown_requested_at = NULL,
            updated_at = now()
        WHERE uuid = $1
        "#,
    )
    .bind(node_uuid)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn remove_run(pool: &PgPool, run_id: i32) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        r#"
        WITH RECURSIVE deleted_runs AS (
            SELECT id
            FROM runs
            WHERE id = $1
            UNION ALL
            SELECT child.id
            FROM runs child
            JOIN deleted_runs parent ON child.parent_run_id = parent.id
        )
        UPDATE nodes
        SET
            desired_run_id = CASE
                WHEN desired_run_id IN (SELECT id FROM deleted_runs) THEN NULL
                ELSE desired_run_id
            END,
            desired_role = CASE
                WHEN desired_run_id IN (SELECT id FROM deleted_runs) THEN NULL
                ELSE desired_role
            END,
            active_run_id = CASE
                WHEN active_run_id IN (SELECT id FROM deleted_runs) THEN NULL
                ELSE active_run_id
            END,
            active_role = CASE
                WHEN active_run_id IN (SELECT id FROM deleted_runs) THEN NULL
                ELSE active_role
            END,
            updated_at = now()
        WHERE desired_run_id IN (SELECT id FROM deleted_runs)
           OR active_run_id IN (SELECT id FROM deleted_runs)
        "#,
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    let result = sqlx::query(
        r#"
        WITH RECURSIVE deleted_runs AS (
            SELECT id
            FROM runs
            WHERE id = $1
            UNION ALL
            SELECT child.id
            FROM runs child
            JOIN deleted_runs parent ON child.parent_run_id = parent.id
        )
        DELETE FROM runs
        WHERE id IN (SELECT id FROM deleted_runs)
        "#,
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(result.rows_affected())
}
