use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) async fn load_run_spec_payload(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<(JsonValue, JsonValue)>, sqlx::Error> {
    let payload = sqlx::query_as::<_, (JsonValue, JsonValue)>(
        r#"
        SELECT
            COALESCE(integration_params, '{}'::jsonb) AS integration_params,
            point_spec AS domain
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(payload)
}

pub(crate) async fn set_run_parent_metadata(
    pool: &PgPool,
    run_id: i32,
    parent_run_id: i32,
    parent_task_id: Option<i64>,
    spawn_kind: &str,
    spawn_label: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE runs
        SET
            parent_run_id = $2,
            parent_task_id = $3,
            spawn_kind = $4,
            spawn_label = $5
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(parent_run_id)
    .bind(parent_task_id)
    .bind(spawn_kind)
    .bind(spawn_label)
    .execute(pool)
    .await?;
    Ok(())
}
