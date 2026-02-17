//! SQL query wrappers and inline statements.

use crate::batch::{Batch, BatchResults};
use crate::models::{AggregatedResult, RunProgress, WorkQueueStats};
use serde_json::Value as JsonValue;
use sqlx::{PgPool, Row};

pub(crate) async fn health_check(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").fetch_one(pool).await?;
    Ok(())
}

pub(crate) async fn get_all_runs(pool: &PgPool) -> Result<Vec<RunProgress>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            run_id,
            run_status,
            started_at,
            completed_at,
            total_batches_planned,
            batches_completed,
            final_result,
            error_estimate,
            total_batches,
            total_samples,
            pending_batches,
            claimed_batches,
            completed_batches,
            failed_batches,
            completion_rate
        FROM run_progress
        ORDER BY started_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut runs = Vec::new();
    for row in rows {
        runs.push(RunProgress {
            run_id: row.get("run_id"),
            run_status: row.get("run_status"),
            started_at: row.get("started_at"),
            completed_at: row.get("completed_at"),
            total_batches_planned: row.get("total_batches_planned"),
            batches_completed: row.get("batches_completed"),
            final_result: row.get("final_result"),
            error_estimate: row.get("error_estimate"),
            total_batches: row.get("total_batches"),
            total_samples: row.get("total_samples"),
            pending_batches: row.get("pending_batches"),
            claimed_batches: row.get("claimed_batches"),
            completed_batches: row.get("completed_batches"),
            failed_batches: row.get("failed_batches"),
            completion_rate: row.get("completion_rate"),
        });
    }

    Ok(runs)
}

pub(crate) async fn get_run_progress(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<RunProgress>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            run_id,
            run_status,
            started_at,
            completed_at,
            total_batches_planned,
            batches_completed,
            final_result,
            error_estimate,
            total_batches,
            total_samples,
            pending_batches,
            claimed_batches,
            completed_batches,
            failed_batches,
            completion_rate
        FROM run_progress
        WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| RunProgress {
        run_id: r.get("run_id"),
        run_status: r.get("run_status"),
        started_at: r.get("started_at"),
        completed_at: r.get("completed_at"),
        total_batches_planned: r.get("total_batches_planned"),
        batches_completed: r.get("batches_completed"),
        final_result: r.get("final_result"),
        error_estimate: r.get("error_estimate"),
        total_batches: r.get("total_batches"),
        total_samples: r.get("total_samples"),
        pending_batches: r.get("pending_batches"),
        claimed_batches: r.get("claimed_batches"),
        completed_batches: r.get("completed_batches"),
        failed_batches: r.get("failed_batches"),
        completion_rate: r.get("completion_rate"),
    }))
}

pub(crate) async fn get_work_queue_stats(
    pool: &PgPool,
    run_id: i32,
) -> Result<Vec<WorkQueueStats>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            run_id,
            status,
            batch_count,
            total_samples,
            avg_batch_time_ms,
            avg_sample_time_ms
        FROM work_queue_stats
        WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    let mut stats = Vec::new();
    for row in rows {
        stats.push(WorkQueueStats {
            run_id: row.get("run_id"),
            status: row.get("status"),
            batch_count: row.get("batch_count"),
            total_samples: row.get("total_samples"),
            avg_batch_time_ms: row.get("avg_batch_time_ms"),
            avg_sample_time_ms: row.get("avg_sample_time_ms"),
        });
    }

    Ok(stats)
}

pub(crate) async fn get_latest_aggregated_result(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<AggregatedResult>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT
            id,
            run_id,
            nr_samples,
            nr_batches,
            sum,
            sum_x2,
            sum_abs,
            max,
            min,
            weighted_sum,
            weighted_sum_x2,
            sum_weights,
            effective_sample_size,
            mean,
            variance,
            std_dev,
            error_estimate,
            histograms,
            created_at
        FROM aggregated_results
        WHERE run_id = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| AggregatedResult {
        id: r.get("id"),
        run_id: r.get("run_id"),
        nr_samples: r.get("nr_samples"),
        nr_batches: r.get("nr_batches"),
        sum: r.get("sum"),
        sum_x2: r.get("sum_x2"),
        sum_abs: r.get("sum_abs"),
        max: r.get("max"),
        min: r.get("min"),
        weighted_sum: r.get("weighted_sum"),
        weighted_sum_x2: r.get("weighted_sum_x2"),
        sum_weights: r.get("sum_weights"),
        effective_sample_size: r.get("effective_sample_size"),
        mean: r.get("mean"),
        variance: r.get("variance"),
        std_dev: r.get("std_dev"),
        error_estimate: r.get("error_estimate"),
        histograms: r.get("histograms"),
        created_at: r.get("created_at"),
    }))
}

pub(crate) async fn get_aggregated_results(
    pool: &PgPool,
    run_id: i32,
    limit: i64,
) -> Result<Vec<AggregatedResult>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            run_id,
            nr_samples,
            nr_batches,
            sum,
            sum_x2,
            sum_abs,
            max,
            min,
            weighted_sum,
            weighted_sum_x2,
            sum_weights,
            effective_sample_size,
            mean,
            variance,
            std_dev,
            error_estimate,
            histograms,
            created_at
        FROM aggregated_results
        WHERE run_id = $1
        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(run_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let mut results = Vec::new();
    for row in rows {
        results.push(AggregatedResult {
            id: row.get("id"),
            run_id: row.get("run_id"),
            nr_samples: row.get("nr_samples"),
            nr_batches: row.get("nr_batches"),
            sum: row.get("sum"),
            sum_x2: row.get("sum_x2"),
            sum_abs: row.get("sum_abs"),
            max: row.get("max"),
            min: row.get("min"),
            weighted_sum: row.get("weighted_sum"),
            weighted_sum_x2: row.get("weighted_sum_x2"),
            sum_weights: row.get("sum_weights"),
            effective_sample_size: row.get("effective_sample_size"),
            mean: row.get("mean"),
            variance: row.get("variance"),
            std_dev: row.get("std_dev"),
            error_estimate: row.get("error_estimate"),
            histograms: row.get("histograms"),
            created_at: row.get("created_at"),
        });
    }

    Ok(results)
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

pub(crate) async fn claim_batch(
    pool: &PgPool,
    run_id: i32,
    worker_id: &str,
) -> Result<Option<(i64, Batch)>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        UPDATE batches
        SET status = 'claimed',
            claimed_by = $1,
            claimed_at = now()
        WHERE id IN (
            SELECT id FROM batches
            WHERE run_id = $2
              AND status = 'pending'
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

    if let Some(row) = row {
        let batch_id: i64 = row.get("id");
        let points_json: JsonValue = row.get("points");
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
    eval_time_ms: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE batches
        SET status = 'completed',
            results = $1,
            total_eval_time_ms = $2,
            completed_at = now()
        WHERE id = $3
        "#,
    )
    .bind(results.to_json())
    .bind(eval_time_ms)
    .bind(batch_id)
    .execute(pool)
    .await?;

    Ok(())
}

// ---- Sampler-aggregator (DB wrappers) ---------------------------------------

pub(crate) async fn get_latest_aggregation_snapshot(
    pool: &PgPool,
    run_id: i32,
) -> Result<
    Option<(
        i64,
        i32,
        f64,
        f64,
        f64,
        Option<f64>,
        Option<f64>,
        f64,
        f64,
        f64,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<f64>,
        chrono::DateTime<chrono::Utc>,
    )>,
    sqlx::Error,
> {
    let row = sqlx::query(
        r#"
        SELECT
            nr_samples,
            nr_batches,
            sum,
            sum_x2,
            sum_abs,
            max,
            min,
            weighted_sum,
            weighted_sum_x2,
            sum_weights,
            mean,
            variance,
            std_dev,
            error_estimate,
            created_at
        FROM aggregated_results
        WHERE run_id = $1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        (
            r.get("nr_samples"),
            r.get("nr_batches"),
            r.get("sum"),
            r.get("sum_x2"),
            r.get("sum_abs"),
            r.get("max"),
            r.get("min"),
            r.get("weighted_sum"),
            r.get("weighted_sum_x2"),
            r.get("sum_weights"),
            r.get("mean"),
            r.get("variance"),
            r.get("std_dev"),
            r.get("error_estimate"),
            r.get("created_at"),
        )
    }))
}

pub(crate) async fn insert_aggregated_results_snapshot(
    pool: &PgPool,
    run_id: i32,
    nr_samples: i64,
    nr_batches: i32,
    sum: f64,
    sum_x2: f64,
    sum_abs: f64,
    max: Option<f64>,
    min: Option<f64>,
    weighted_sum: f64,
    weighted_sum_x2: f64,
    sum_weights: f64,
    mean: Option<f64>,
    variance: Option<f64>,
    std_dev: Option<f64>,
    error_estimate: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO aggregated_results (
            run_id,
            nr_samples,
            nr_batches,
            sum,
            sum_x2,
            sum_abs,
            max,
            min,
            weighted_sum,
            weighted_sum_x2,
            sum_weights,
            effective_sample_size,
            mean,
            variance,
            std_dev,
            error_estimate,
            histograms
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8,
            $9, $10, $11, $12, $13, $14, $15, $16, $17
        )
        "#,
    )
    .bind(run_id)
    .bind(nr_samples)
    .bind(nr_batches)
    .bind(sum)
    .bind(sum_x2)
    .bind(sum_abs)
    .bind(max)
    .bind(min)
    .bind(weighted_sum)
    .bind(weighted_sum_x2)
    .bind(sum_weights)
    .bind::<Option<f64>>(None)
    .bind(mean)
    .bind(variance)
    .bind(std_dev)
    .bind(error_estimate)
    .bind::<Option<JsonValue>>(None)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn update_run_summary_from_snapshot(
    pool: &PgPool,
    run_id: i32,
    delta_batches_completed: i32,
    final_result: Option<f64>,
    error_estimate: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE runs
        SET
            batches_completed = COALESCE(batches_completed, 0) + $1,
            final_result = $2,
            error_estimate = $3
        WHERE id = $4
        "#,
    )
    .bind(delta_batches_completed)
    .bind(final_result)
    .bind(error_estimate)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}
