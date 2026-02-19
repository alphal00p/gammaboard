use crate::batch::PointSpec;
use crate::core::{RunStatus, WorkerRole};
use serde_json::Value as JsonValue;
use sqlx::PgPool;

fn control_plane_worker_id(node_id: &str, role: WorkerRole) -> String {
    format!("{node_id}-{role}")
}

pub(crate) struct DesiredAssignmentRaw {
    pub node_id: String,
    pub role: String,
    pub run_id: i32,
}

pub(crate) async fn upsert_desired_assignment(
    pool: &PgPool,
    node_id: &str,
    role: WorkerRole,
    run_id: i32,
) -> Result<(), sqlx::Error> {
    let worker_id = control_plane_worker_id(node_id, role);
    sqlx::query(
        r#"
        INSERT INTO workers (
            worker_id,
            node_id,
            role,
            implementation,
            version,
            node_specs,
            status,
            desired_run_id,
            desired_updated_at,
            updated_at
        ) VALUES (
            $1, $2, $3, 'control_plane', 'v1', '{}'::jsonb, 'inactive', $4, now(), now()
        )
        ON CONFLICT (worker_id) DO UPDATE
        SET
            node_id = EXCLUDED.node_id,
            role = EXCLUDED.role,
            desired_run_id = EXCLUDED.desired_run_id,
            desired_updated_at = now(),
            updated_at = now()
        "#,
    )
    .bind(&worker_id)
    .bind(node_id)
    .bind(role.as_str())
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn clear_desired_assignment(
    pool: &PgPool,
    node_id: &str,
    role: WorkerRole,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE workers
        SET
            desired_run_id = NULL,
            desired_updated_at = now(),
            updated_at = now()
        WHERE node_id = $1 AND role = $2
        "#,
    )
    .bind(node_id)
    .bind(role.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn get_desired_assignment_run_id(
    pool: &PgPool,
    node_id: &str,
    role: WorkerRole,
) -> Result<Option<i32>, sqlx::Error> {
    let run_id = sqlx::query_scalar(
        r#"
        SELECT desired_run_id AS run_id
        FROM workers
        WHERE node_id = $1 AND role = $2
          AND desired_run_id IS NOT NULL
        "#,
    )
    .bind(node_id)
    .bind(role.as_str())
    .fetch_optional(pool)
    .await?;
    Ok(run_id)
}

pub(crate) async fn list_desired_assignments(
    pool: &PgPool,
    node_id: Option<&str>,
) -> Result<Vec<DesiredAssignmentRaw>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, i32)>(
        r#"
        SELECT node_id, role, desired_run_id AS run_id
        FROM workers
        WHERE desired_run_id IS NOT NULL
          AND ($1::text IS NULL OR node_id = $1)
        ORDER BY node_id ASC, role ASC
        "#,
    )
    .bind(node_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(node_id, role, run_id)| DesiredAssignmentRaw {
            node_id,
            role,
            run_id,
        })
        .collect())
}

pub(crate) async fn create_run(
    pool: &PgPool,
    status: RunStatus,
    integration_params: &JsonValue,
    point_spec: &PointSpec,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        INSERT INTO runs (status, integration_params, point_spec)
        VALUES ($1, $2, $3)
        RETURNING id
        "#,
    )
    .bind(status.as_str())
    .bind(integration_params)
    .bind(sqlx::types::Json(point_spec))
    .fetch_one(pool)
    .await
}

pub(crate) async fn set_run_status(
    pool: &PgPool,
    run_id: i32,
    status: RunStatus,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE runs
        SET status = $2
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(status.as_str())
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
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
