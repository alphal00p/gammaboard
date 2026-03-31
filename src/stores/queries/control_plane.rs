use crate::core::WorkerRole;
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
    pub desired_role: Option<String>,
    pub desired_run_id: Option<i32>,
    pub desired_run_name: Option<String>,
    pub current_role: Option<String>,
    pub current_run_id: Option<i32>,
    pub current_run_name: Option<String>,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
}

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
) -> Result<(), sqlx::Error> {
    let row = sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO nodes (
            name,
            uuid,
            lease_expires_at,
            last_seen,
            updated_at
        ) VALUES (
            $1,
            $2,
            now() + interval '10 seconds',
            now(),
            now()
        )
        ON CONFLICT (name) DO UPDATE
        SET
            uuid = EXCLUDED.uuid,
            lease_expires_at = EXCLUDED.lease_expires_at,
            last_seen = EXCLUDED.last_seen,
            updated_at = EXCLUDED.updated_at,
            active_run_id = CASE
                WHEN nodes.uuid = EXCLUDED.uuid THEN nodes.active_run_id
                WHEN nodes.lease_expires_at <= now() THEN NULL
                ELSE nodes.active_run_id
            END,
            active_role = CASE
                WHEN nodes.uuid = EXCLUDED.uuid THEN nodes.active_role
                WHEN nodes.lease_expires_at <= now() THEN NULL
                ELSE nodes.active_role
            END
        WHERE nodes.uuid = EXCLUDED.uuid OR nodes.lease_expires_at <= now()
        RETURNING 1
        "#,
    )
    .bind(node_name)
    .bind(node_uuid)
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
    let result = sqlx::query(
        r#"
        UPDATE nodes
        SET
            active_run_id = $2,
            active_role = $3,
            updated_at = now()
        WHERE uuid = $1
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
            shutdown_requested_at,
            updated_at
        )
        VALUES
            ($1, '', to_timestamp(0), now(), now())
        ON CONFLICT (name) DO UPDATE
        SET
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
            shutdown_requested_at = now(),
            updated_at = now()
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
    let result = sqlx::query(
        r#"
        DELETE FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}
