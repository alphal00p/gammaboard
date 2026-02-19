use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) async fn load_integration_params(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<JsonValue>, sqlx::Error> {
    let params = sqlx::query_scalar::<_, JsonValue>(
        r#"
        SELECT COALESCE(integration_params, '{}'::jsonb)
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(params)
}
