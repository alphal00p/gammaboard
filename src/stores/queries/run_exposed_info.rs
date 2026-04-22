use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) async fn get_run_exposed_info(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<JsonValue>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT exposed_info
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten())
}

pub(crate) async fn set_run_exposed_info(
    pool: &PgPool,
    run_id: i32,
    value: &JsonValue,
) -> Result<u64, sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE runs
        SET exposed_info = $2
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(value)
    .execute(pool)
    .await
    .map(|result| result.rows_affected())
}
