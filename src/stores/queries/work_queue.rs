use crate::core::{Batch, BatchResult};
use crate::core::{EvaluatorPerformanceSnapshot, SamplerAggregatorPerformanceSnapshot};
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::PgPool;

pub(crate) struct CompletedBatchRaw {
    pub batch_id: i64,
    pub points: JsonValue,
    pub requires_training: bool,
    pub values: Option<JsonValue>,
    pub batch_observable: JsonValue,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_eval_time_ms: Option<f64>,
}

pub(crate) async fn insert_batch(
    pool: &PgPool,
    run_id: i32,
    batch: &Batch,
    requires_training: bool,
) -> Result<i64, sqlx::Error> {
    let batch_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO batches (run_id, points, batch_size, status, requires_training)
        VALUES ($1, $2, $3, 'pending', $4)
        RETURNING id
        "#,
    )
    .bind(run_id)
    .bind(batch.to_json())
    .bind(batch.size() as i32)
    .bind(requires_training)
    .fetch_one(pool)
    .await?;
    Ok(batch_id)
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
) -> Result<Option<(i64, Batch, bool)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, JsonValue, bool)>(
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
        RETURNING id, points, requires_training
        "#,
    )
    .bind(worker_id)
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    if let Some((batch_id, points_json, requires_training)) = row {
        let batch = Batch::from_json(&points_json).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(Some((batch_id, batch, requires_training)))
    } else {
        Ok(None)
    }
}

pub(crate) async fn release_claimed_batches_for_worker(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE batches
        SET status = 'pending',
            claimed_by = NULL,
            claimed_at = NULL
        WHERE run_id = $1
          AND status = 'claimed'
          AND claimed_by = $2
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn submit_batch_results(
    pool: &PgPool,
    batch_id: i64,
    result: &BatchResult,
    eval_time_ms: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE batches
        SET status = 'completed',
            "values" = $1,
            batch_observable = $2,
            total_eval_time_ms = $3,
            completed_at = now()
        WHERE id = $4
        "#,
    )
    .bind(result.values_to_json())
    .bind(serde_json::to_value(&result.observable).unwrap_or_default())
    .bind(eval_time_ms)
    .bind(batch_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn insert_evaluator_performance_snapshot(
    pool: &PgPool,
    snapshot: &EvaluatorPerformanceSnapshot,
) -> Result<(), sqlx::Error> {
    let metrics = serde_json::to_value(&snapshot.metrics).unwrap_or_default();
    sqlx::query(
        r#"
        INSERT INTO evaluator_performance_history (
            run_id,
            worker_id,
            metrics
        )
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(snapshot.run_id)
    .bind(&snapshot.worker_id)
    .bind(&metrics)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn insert_sampler_aggregator_performance_snapshot(
    pool: &PgPool,
    snapshot: &SamplerAggregatorPerformanceSnapshot,
) -> Result<(), sqlx::Error> {
    let metrics =
        serde_json::to_value(snapshot.runtime_metrics.to_performance_metrics()).unwrap_or_default();
    let runtime_metrics = serde_json::to_value(&snapshot.runtime_metrics).unwrap_or_default();
    sqlx::query(
        r#"
        INSERT INTO sampler_aggregator_performance_history (
            run_id,
            worker_id,
            metrics,
            runtime_metrics,
            engine_diagnostics
        )
        VALUES (
            $1,
            $2,
            $3,
            $4,
            $5
        )
        "#,
    )
    .bind(snapshot.run_id)
    .bind(&snapshot.worker_id)
    .bind(&metrics)
    .bind(&runtime_metrics)
    .bind(&snapshot.engine_diagnostics)
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

pub(crate) async fn fetch_completed_batches(
    pool: &PgPool,
    run_id: i32,
    limit: usize,
) -> Result<Vec<CompletedBatchRaw>, sqlx::Error> {
    // Return only the contiguous completed prefix by id.
    // This keeps ingestion strictly ordered across batches and leaves out-of-order
    // completions buffered in the DB until gaps are resolved.
    let rows = sqlx::query_as::<
        _,
        (
            i64,
            JsonValue,
            bool,
            Option<JsonValue>,
            JsonValue,
            Option<DateTime<Utc>>,
            Option<f64>,
        ),
    >(
        r#"
        WITH ordered AS (
            SELECT
                id,
                status,
                points,
                requires_training,
                "values",
                batch_observable,
                completed_at,
                total_eval_time_ms,
                ROW_NUMBER() OVER (ORDER BY id ASC) AS rn
            FROM batches
            WHERE run_id = $1
            ORDER BY id ASC
            LIMIT $2
        ),
        first_blocker AS (
            SELECT MIN(rn) AS rn
            FROM ordered
            WHERE status <> 'completed'
        )
        SELECT
            o.id,
            o.points,
            o.requires_training,
            o."values",
            o.batch_observable,
            o.completed_at,
            o.total_eval_time_ms
        FROM ordered o
        CROSS JOIN first_blocker b
        WHERE o.status = 'completed'
          AND o.batch_observable IS NOT NULL
          AND (o.requires_training = false OR o."values" IS NOT NULL)
          AND (b.rn IS NULL OR o.rn < b.rn)
        ORDER BY o.id ASC
        "#,
    )
    .bind(run_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                batch_id,
                points,
                requires_training,
                values,
                batch_observable,
                completed_at,
                total_eval_time_ms,
            )| {
                CompletedBatchRaw {
                    batch_id,
                    points,
                    requires_training,
                    values,
                    batch_observable,
                    completed_at,
                    total_eval_time_ms,
                }
            },
        )
        .collect())
}

pub(crate) async fn delete_completed_batches(
    pool: &PgPool,
    batch_ids: &[i64],
) -> Result<(), sqlx::Error> {
    if batch_ids.is_empty() {
        return Ok(());
    }

    sqlx::query(
        r#"
        DELETE FROM batches
        WHERE status = 'completed'
          AND id = ANY($1)
        "#,
    )
    .bind(batch_ids)
    .execute(pool)
    .await?;

    Ok(())
}
