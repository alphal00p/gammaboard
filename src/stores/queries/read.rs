use crate::models::{AggregatedResult, RunProgress, RunStatus, WorkQueueStats};
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::PgPool;

fn parse_run_status(value: &str) -> Result<RunStatus, sqlx::Error> {
    RunStatus::from_db(value).ok_or_else(|| {
        sqlx::Error::Decode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("unknown run status: {value}"),
        )))
    })
}

#[derive(sqlx::FromRow)]
struct RunProgressRow {
    run_id: i32,
    run_status: String,
    integration_params: Option<JsonValue>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    total_batches_planned: Option<i32>,
    batches_completed: i32,
    final_result: Option<f64>,
    error_estimate: Option<f64>,
    total_batches: i64,
    total_samples: i64,
    pending_batches: i64,
    claimed_batches: i64,
    completed_batches: i64,
    failed_batches: i64,
    completion_rate: f64,
}

impl TryFrom<RunProgressRow> for RunProgress {
    type Error = sqlx::Error;

    fn try_from(value: RunProgressRow) -> Result<Self, Self::Error> {
        Ok(RunProgress {
            run_id: value.run_id,
            run_status: parse_run_status(&value.run_status)?,
            integration_params: value.integration_params,
            started_at: value.started_at,
            completed_at: value.completed_at,
            total_batches_planned: value.total_batches_planned,
            batches_completed: value.batches_completed,
            final_result: value.final_result,
            error_estimate: value.error_estimate,
            total_batches: value.total_batches,
            total_samples: value.total_samples,
            pending_batches: value.pending_batches,
            claimed_batches: value.claimed_batches,
            completed_batches: value.completed_batches,
            failed_batches: value.failed_batches,
            completion_rate: value.completion_rate,
        })
    }
}

#[derive(sqlx::FromRow)]
struct AggregatedResultRow {
    id: i64,
    run_id: i32,
    aggregated_observable: JsonValue,
    created_at: Option<DateTime<Utc>>,
}

impl From<AggregatedResultRow> for AggregatedResult {
    fn from(value: AggregatedResultRow) -> Self {
        Self {
            id: value.id,
            run_id: value.run_id,
            aggregated_observable: value.aggregated_observable,
            created_at: value.created_at,
        }
    }
}

pub(crate) async fn health_check(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").fetch_one(pool).await?;
    Ok(())
}

pub(crate) async fn get_all_runs(pool: &PgPool) -> Result<Vec<RunProgress>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RunProgressRow>(
        r#"
        SELECT
            run_id,
            run_status,
            integration_params,
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

    rows.into_iter().map(TryInto::try_into).collect()
}

pub(crate) async fn get_run_progress(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<RunProgress>, sqlx::Error> {
    let row = sqlx::query_as::<_, RunProgressRow>(
        r#"
        SELECT
            run_id,
            run_status,
            integration_params,
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

    row.map(TryInto::try_into).transpose()
}

pub(crate) async fn get_work_queue_stats(
    pool: &PgPool,
    run_id: i32,
) -> Result<Vec<WorkQueueStats>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i32, String, i64, i64, Option<f64>, Option<f64>)>(
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
    for (run_id, status, batch_count, total_samples, avg_batch_time_ms, avg_sample_time_ms) in rows
    {
        stats.push(WorkQueueStats {
            run_id,
            status,
            batch_count,
            total_samples,
            avg_batch_time_ms,
            avg_sample_time_ms,
        });
    }

    Ok(stats)
}

pub(crate) async fn get_latest_aggregated_result(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<AggregatedResult>, sqlx::Error> {
    let row = sqlx::query_as::<_, AggregatedResultRow>(
        r#"
        SELECT
            id,
            run_id,
            aggregated_observable,
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

    Ok(row.map(Into::into))
}

pub(crate) async fn get_aggregated_results(
    pool: &PgPool,
    run_id: i32,
    limit: i64,
) -> Result<Vec<AggregatedResult>, sqlx::Error> {
    let rows = sqlx::query_as::<_, AggregatedResultRow>(
        r#"
        SELECT
            id,
            run_id,
            aggregated_observable,
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

    Ok(rows.into_iter().map(Into::into).collect())
}
