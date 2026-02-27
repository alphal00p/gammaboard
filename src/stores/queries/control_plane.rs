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

pub(crate) async fn clear_desired_assignments_for_run(
    pool: &PgPool,
    run_id: i32,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE workers
        SET
            desired_run_id = NULL,
            desired_updated_at = now(),
            updated_at = now()
        WHERE desired_run_id = $1
        "#,
    )
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn clear_all_desired_assignments(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE workers
        SET
            desired_run_id = NULL,
            desired_updated_at = now(),
            updated_at = now()
        WHERE desired_run_id IS NOT NULL
        "#,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
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

pub(crate) async fn request_node_shutdown(
    pool: &PgPool,
    node_id: &str,
) -> Result<u64, sqlx::Error> {
    let evaluator_worker_id = control_plane_worker_id(node_id, WorkerRole::Evaluator);
    let sampler_worker_id = control_plane_worker_id(node_id, WorkerRole::SamplerAggregator);

    let result = sqlx::query(
        r#"
        INSERT INTO workers (
            worker_id,
            node_id,
            role,
            implementation,
            version,
            node_specs,
            status,
            shutdown_requested_at,
            updated_at
        )
        VALUES
            ($1, $2, 'evaluator', 'control_plane', 'v1', '{}'::jsonb, 'inactive', now(), now()),
            ($3, $2, 'sampler_aggregator', 'control_plane', 'v1', '{}'::jsonb, 'inactive', now(), now())
        ON CONFLICT (worker_id) DO UPDATE
        SET
            node_id = EXCLUDED.node_id,
            role = EXCLUDED.role,
            shutdown_requested_at = now(),
            updated_at = now()
        "#,
    )
    .bind(evaluator_worker_id)
    .bind(node_id)
    .bind(sampler_worker_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub(crate) async fn request_all_nodes_shutdown(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE workers
        SET
            shutdown_requested_at = now(),
            updated_at = now()
        WHERE node_id IS NOT NULL
        "#,
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub(crate) async fn consume_node_shutdown_request(
    pool: &PgPool,
    node_id: &str,
) -> Result<bool, sqlx::Error> {
    let requested = sqlx::query_scalar(
        r#"
        WITH cleared AS (
            UPDATE workers
            SET
                shutdown_requested_at = NULL,
                updated_at = now()
            WHERE node_id = $1
              AND shutdown_requested_at IS NOT NULL
            RETURNING 1
        )
        SELECT EXISTS(SELECT 1 FROM cleared)
        "#,
    )
    .bind(node_id)
    .fetch_one(pool)
    .await?;

    Ok(requested)
}

pub(crate) async fn create_run(
    pool: &PgPool,
    status: RunStatus,
    name: &str,
    integration_params: &JsonValue,
    observable_implementation: &str,
    point_spec: &PointSpec,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        INSERT INTO runs (status, name, integration_params, observable_implementation, point_spec)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(status.as_str())
    .bind(name)
    .bind(integration_params)
    .bind(observable_implementation)
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

pub(crate) async fn set_all_runs_status(
    pool: &PgPool,
    status: RunStatus,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE runs
        SET status = $1
        "#,
    )
    .bind(status.as_str())
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn try_set_evaluator_init_metadata(
    pool: &PgPool,
    run_id: i32,
    metadata: &JsonValue,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE runs
        SET evaluator_init_metadata = $2
        WHERE id = $1
          AND evaluator_init_metadata IS NULL
        "#,
    )
    .bind(run_id)
    .bind(metadata)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn try_set_sampler_init_metadata(
    pool: &PgPool,
    run_id: i32,
    metadata: &JsonValue,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE runs
        SET sampler_aggregator_init_metadata = $2
        WHERE id = $1
          AND sampler_aggregator_init_metadata IS NULL
        "#,
    )
    .bind(run_id)
    .bind(metadata)
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
