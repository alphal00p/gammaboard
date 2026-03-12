use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) async fn load_run_spec_payload(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<(JsonValue, JsonValue, Option<i64>)>, sqlx::Error> {
    let payload = sqlx::query_as::<_, (JsonValue, JsonValue, Option<i64>)>(
        r#"
        SELECT
            COALESCE(integration_params, '{}'::jsonb) AS integration_params,
            point_spec,
            target_nr_samples
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(payload)
}
