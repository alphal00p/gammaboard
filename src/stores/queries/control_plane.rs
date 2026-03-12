use crate::core::PointSpec;
use crate::core::WorkerRole;
use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) struct DesiredAssignmentRaw {
    pub node_id: String,
    pub role: String,
    pub run_id: i32,
}

pub(crate) struct NodeRaw {
    pub node_id: String,
    pub desired_role: Option<String>,
    pub desired_run_id: Option<i32>,
    pub current_role: Option<String>,
    pub current_run_id: Option<i32>,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
}

pub(crate) async fn upsert_desired_assignment(
    pool: &PgPool,
    node_id: &str,
    role: WorkerRole,
    run_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO nodes (
            node_id,
            desired_run_id,
            desired_role,
            updated_at
        ) VALUES (
            $1, $2, $3, now()
        )
        ON CONFLICT (node_id) DO UPDATE
        SET
            desired_run_id = EXCLUDED.desired_run_id,
            desired_role = EXCLUDED.desired_role,
            updated_at = now()
        "#,
    )
    .bind(node_id)
    .bind(run_id)
    .bind(role.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn register_node(pool: &PgPool, node_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO nodes (
            node_id,
            last_seen,
            updated_at
        ) VALUES (
            $1, now(), now()
        )
        ON CONFLICT (node_id) DO UPDATE
        SET
            last_seen = now(),
            updated_at = now()
        "#,
    )
    .bind(node_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn heartbeat_node(pool: &PgPool, node_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE nodes
        SET last_seen = now(), updated_at = now()
        WHERE node_id = $1
        "#,
    )
    .bind(node_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn clear_desired_assignment(
    pool: &PgPool,
    node_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE nodes
        SET
            desired_run_id = NULL,
            desired_role = NULL,
            updated_at = now()
        WHERE node_id = $1
        "#,
    )
    .bind(node_id)
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
        UPDATE nodes
        SET
            desired_run_id = NULL,
            desired_role = NULL,
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
        UPDATE nodes
        SET
            desired_run_id = NULL,
            desired_role = NULL,
            updated_at = now()
        "#,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn get_desired_assignment(
    pool: &PgPool,
    node_id: &str,
) -> Result<Option<DesiredAssignmentRaw>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, String, i32)>(
        r#"
        SELECT node_id, desired_role AS role, desired_run_id AS run_id
        FROM nodes
        WHERE node_id = $1
          AND desired_run_id IS NOT NULL
        LIMIT 1
        "#,
    )
    .bind(node_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(node_id, role, run_id)| DesiredAssignmentRaw {
        node_id,
        role,
        run_id,
    }))
}

pub(crate) async fn list_desired_assignments(
    pool: &PgPool,
    node_id: Option<&str>,
) -> Result<Vec<DesiredAssignmentRaw>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, i32)>(
        r#"
        SELECT node_id, desired_role AS role, desired_run_id AS run_id
        FROM nodes
        WHERE desired_run_id IS NOT NULL
          AND ($1::text IS NULL OR node_id = $1)
        ORDER BY node_id ASC
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

pub(crate) async fn list_nodes(
    pool: &PgPool,
    node_id: Option<&str>,
) -> Result<Vec<NodeRaw>, sqlx::Error> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<i32>,
            Option<String>,
            Option<i32>,
            Option<chrono::DateTime<chrono::Utc>>,
        ),
    >(
        r#"
        SELECT
            n.node_id,
            n.desired_role,
            n.desired_run_id,
            n.current_role,
            n.current_run_id,
            n.last_seen
        FROM nodes n
        WHERE ($1::text IS NULL OR n.node_id = $1)
        ORDER BY n.node_id ASC
        "#,
    )
    .bind(node_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(node_id, desired_role, desired_run_id, current_role, current_run_id, last_seen)| {
                NodeRaw {
                    node_id,
                    desired_role,
                    desired_run_id,
                    current_role,
                    current_run_id,
                    last_seen,
                }
            },
        )
        .collect())
}

pub(crate) async fn set_current_assignment(
    pool: &PgPool,
    node_id: &str,
    role: WorkerRole,
    run_id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE nodes
        SET
            current_run_id = $2,
            current_role = $3,
            updated_at = now()
        WHERE node_id = $1
        "#,
    )
    .bind(node_id)
    .bind(run_id)
    .bind(role.as_str())
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn clear_current_assignment(
    pool: &PgPool,
    node_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE nodes
        SET
            current_run_id = NULL,
            current_role = NULL,
            updated_at = now()
        WHERE node_id = $1
        "#,
    )
    .bind(node_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn request_node_shutdown(
    pool: &PgPool,
    node_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        INSERT INTO nodes (
            node_id,
            shutdown_requested_at,
            updated_at
        )
        VALUES
            ($1, now(), now())
        ON CONFLICT (node_id) DO UPDATE
        SET
            shutdown_requested_at = now(),
            updated_at = now()
        "#,
    )
    .bind(node_id)
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
    node_id: &str,
) -> Result<bool, sqlx::Error> {
    let requested = sqlx::query_scalar(
        r#"
        WITH cleared AS (
            UPDATE nodes
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
    name: &str,
    integration_params: &JsonValue,
    target: Option<&JsonValue>,
    point_spec: &PointSpec,
    evaluator_init_metadata: Option<&JsonValue>,
    sampler_aggregator_init_metadata: Option<&JsonValue>,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        INSERT INTO runs (
            name,
            integration_params,
            target,
            point_spec,
            evaluator_init_metadata,
            sampler_aggregator_init_metadata
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(name)
    .bind(integration_params)
    .bind(target)
    .bind(sqlx::types::Json(point_spec))
    .bind(evaluator_init_metadata)
    .bind(sampler_aggregator_init_metadata)
    .fetch_one(pool)
    .await
}

pub(crate) async fn try_set_training_completed_at(
    pool: &PgPool,
    run_id: i32,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE runs
        SET training_completed_at = now()
        WHERE id = $1
          AND training_completed_at IS NULL
        "#,
    )
    .bind(run_id)
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
