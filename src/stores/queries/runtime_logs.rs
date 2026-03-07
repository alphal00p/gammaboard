use crate::core::RuntimeLogEvent;
use sqlx::PgPool;

pub(crate) async fn insert_runtime_log(
    pool: &PgPool,
    event: &RuntimeLogEvent,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO runtime_logs (
            source,
            run_id,
            node_id,
            worker_id,
            level,
            target,
            message,
            fields
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(&event.source)
    .bind(event.run_id)
    .bind(&event.node_id)
    .bind(&event.worker_id)
    .bind(&event.level)
    .bind(&event.target)
    .bind(&event.message)
    .bind(&event.fields)
    .execute(pool)
    .await?;

    Ok(())
}
