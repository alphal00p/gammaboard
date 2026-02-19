use crate::batch::{Batch, BatchResults};
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) struct CompletedBatchRaw {
    pub batch_id: i64,
    pub points: JsonValue,
    pub training_weights: JsonValue,
    pub batch_observable: JsonValue,
    pub completed_at: Option<DateTime<Utc>>,
}

pub(crate) async fn insert_batch(
    pool: &PgPool,
    run_id: i32,
    batch: &Batch,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO batches (run_id, points, batch_size, status)
        VALUES ($1, $2, $3, 'pending')
        "#,
    )
    .bind(run_id)
    .bind(batch.to_json())
    .bind(batch.size() as i32)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn get_pending_batch_count(
    pool: &PgPool,
    run_id: i32,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM batches
        WHERE run_id = $1
          AND status = 'pending'
        "#,
    )
    .bind(run_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub(crate) async fn claim_batch(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
) -> Result<Option<(i64, Batch)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, JsonValue)>(
        r#"
        UPDATE batches
        SET status = 'claimed',
            claimed_by = $1,
            claimed_at = now()
        WHERE id IN (
            SELECT id FROM batches
            WHERE run_id = $2
              AND status = 'pending'
              AND EXISTS (
                  SELECT 1
                  FROM run_evaluator_assignments rea
                  WHERE rea.run_id = $2
                    AND rea.worker_id = $1
                    AND rea.active = true
              )
            ORDER BY created_at
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING id, points
        "#,
    )
    .bind(worker_id)
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    if let Some((batch_id, points_json)) = row {
        let batch = Batch::from_json(&points_json).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(Some((batch_id, batch)))
    } else {
        Ok(None)
    }
}

pub(crate) async fn submit_batch_results(
    pool: &PgPool,
    batch_id: i64,
    results: &BatchResults,
    batch_observable: &JsonValue,
    eval_time_ms: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE batches
        SET status = 'completed',
            training_weights = $1,
            batch_observable = $2,
            total_eval_time_ms = $3,
            completed_at = now()
        WHERE id = $4
        "#,
    )
    .bind(results.to_json())
    .bind(batch_observable)
    .bind(eval_time_ms)
    .bind(batch_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn fail_batch(
    pool: &PgPool,
    batch_id: i64,
    last_error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE batches
        SET
            status = 'failed',
            last_error = $2,
            completed_at = now(),
            retry_count = COALESCE(retry_count, 0) + 1
        WHERE id = $1
        "#,
    )
    .bind(batch_id)
    .bind(last_error)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn fetch_completed_batches_since(
    pool: &PgPool,
    run_id: i32,
    last_batch_id: Option<i64>,
    limit: usize,
) -> Result<Vec<CompletedBatchRaw>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, JsonValue, JsonValue, JsonValue, Option<DateTime<Utc>>)>(
        r#"
        SELECT id, points, training_weights, batch_observable, completed_at
        FROM batches
        WHERE run_id = $1
          AND status = 'completed'
          AND training_weights IS NOT NULL
          AND batch_observable IS NOT NULL
          AND ($2::bigint IS NULL OR id > $2)
        ORDER BY id ASC
        LIMIT $3
        "#,
    )
    .bind(run_id)
    .bind(last_batch_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(batch_id, points, training_weights, batch_observable, completed_at)| {
                CompletedBatchRaw {
                    batch_id,
                    points,
                    training_weights,
                    batch_observable,
                    completed_at,
                }
            },
        )
        .collect())
}
