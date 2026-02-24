use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) async fn load_run_spec_payload(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<(JsonValue, JsonValue)>, sqlx::Error> {
    let payload = sqlx::query_as::<_, (JsonValue, JsonValue)>(
        r#"
        SELECT
            (
                COALESCE(integration_params, '{}'::jsonb)
                || jsonb_build_object('observable_implementation', observable_implementation)
            ) AS integration_params,
            point_spec
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(payload)
}
