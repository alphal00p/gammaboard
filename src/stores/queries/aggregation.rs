use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) async fn get_run_current_observable(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<JsonValue>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT current_observable
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten())
}

pub(crate) async fn get_run_sampler_runner_snapshot(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<JsonValue>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT sampler_runner_snapshot
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten())
}

pub(crate) async fn get_latest_aggregation_snapshot(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<JsonValue>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT aggregated_observable
        FROM aggregated_results
        WHERE run_id = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn insert_aggregated_results_snapshot(
    pool: &PgPool,
    run_id: i32,
    aggregated_observable: &JsonValue,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO aggregated_results (run_id, aggregated_observable)
        VALUES ($1, $2)
        "#,
    )
    .bind(run_id)
    .bind(aggregated_observable)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn update_run_aggregation(
    pool: &PgPool,
    run_id: i32,
    current_observable: &JsonValue,
    delta_batches_completed: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE runs
        SET current_observable = $1,
            batches_completed = COALESCE(batches_completed, 0) + $2
        WHERE id = $3
        "#,
    )
    .bind(current_observable)
    .bind(delta_batches_completed)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn update_run_sampler_runner_snapshot(
    pool: &PgPool,
    run_id: i32,
    snapshot: &JsonValue,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE runs
        SET sampler_runner_snapshot = $1
        WHERE id = $2
        "#,
    )
    .bind(snapshot)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}
