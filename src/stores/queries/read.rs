use crate::core::RunStatus;
use crate::core::{EvaluatorPerformanceMetrics, SamplerPerformanceMetrics};
use crate::stores::{
    AggregatedResult, EvaluatorPerformanceHistoryEntry, RegisteredWorkerEntry, RunProgress,
    SamplerPerformanceHistoryEntry, WorkQueueStats, WorkerLogEntry,
};
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
    run_name: String,
    run_status: String,
    integration_params: Option<JsonValue>,
    target: Option<JsonValue>,
    evaluator_init_metadata: Option<JsonValue>,
    sampler_aggregator_init_metadata: Option<JsonValue>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    training_completed_at: Option<DateTime<Utc>>,
    total_batches_planned: Option<i32>,
    batches_completed: i32,
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
            run_name: value.run_name,
            run_status: parse_run_status(&value.run_status)?,
            integration_params: value.integration_params,
            target: value.target,
            evaluator_init_metadata: value.evaluator_init_metadata,
            sampler_aggregator_init_metadata: value.sampler_aggregator_init_metadata,
            started_at: value.started_at,
            completed_at: value.completed_at,
            training_completed_at: value.training_completed_at,
            total_batches_planned: value.total_batches_planned,
            batches_completed: value.batches_completed,
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

#[derive(sqlx::FromRow)]
struct WorkerLogRow {
    id: i64,
    ts: DateTime<Utc>,
    run_id: Option<i32>,
    node_id: Option<String>,
    worker_id: String,
    role: String,
    level: String,
    event_type: String,
    message: String,
    fields: JsonValue,
}

impl From<WorkerLogRow> for WorkerLogEntry {
    fn from(value: WorkerLogRow) -> Self {
        Self {
            id: value.id,
            ts: value.ts,
            run_id: value.run_id,
            node_id: value.node_id,
            worker_id: value.worker_id,
            role: value.role,
            level: value.level,
            event_type: value.event_type,
            message: value.message,
            fields: value.fields,
        }
    }
}

#[derive(sqlx::FromRow)]
struct RegisteredWorkerRow {
    worker_id: String,
    node_id: Option<String>,
    role: String,
    implementation: String,
    version: String,
    status: String,
    last_seen: Option<DateTime<Utc>>,
    evaluator_metrics: Option<JsonValue>,
    sampler_metrics: Option<JsonValue>,
    evaluator_engine_diagnostics: Option<JsonValue>,
    sampler_runtime_metrics: Option<JsonValue>,
    sampler_engine_diagnostics: Option<JsonValue>,
}

impl From<RegisteredWorkerRow> for RegisteredWorkerEntry {
    fn from(value: RegisteredWorkerRow) -> Self {
        Self {
            worker_id: value.worker_id,
            node_id: value.node_id,
            role: value.role,
            implementation: value.implementation,
            version: value.version,
            status: value.status,
            last_seen: value.last_seen,
            evaluator_metrics: value.evaluator_metrics.and_then(|metrics| {
                serde_json::from_value::<EvaluatorPerformanceMetrics>(metrics).ok()
            }),
            sampler_metrics: value.sampler_metrics.and_then(|metrics| {
                serde_json::from_value::<SamplerPerformanceMetrics>(metrics).ok()
            }),
            evaluator_engine_diagnostics: value.evaluator_engine_diagnostics,
            sampler_runtime_metrics: value.sampler_runtime_metrics,
            sampler_engine_diagnostics: value.sampler_engine_diagnostics,
        }
    }
}

#[derive(sqlx::FromRow)]
struct EvaluatorPerformanceHistoryRow {
    id: i64,
    run_id: i32,
    worker_id: String,
    metrics: JsonValue,
    engine_diagnostics: JsonValue,
    created_at: DateTime<Utc>,
}

impl From<EvaluatorPerformanceHistoryRow> for EvaluatorPerformanceHistoryEntry {
    fn from(value: EvaluatorPerformanceHistoryRow) -> Self {
        Self {
            id: value.id,
            run_id: value.run_id,
            worker_id: value.worker_id,
            metrics: serde_json::from_value::<EvaluatorPerformanceMetrics>(value.metrics)
                .unwrap_or(EvaluatorPerformanceMetrics {
                    batches_completed: 0,
                    samples_evaluated: 0,
                    avg_time_per_sample_ms: 0.0,
                    std_time_per_sample_ms: 0.0,
                    idle_profile: None,
                }),
            engine_diagnostics: value.engine_diagnostics,
            created_at: value.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct SamplerPerformanceHistoryRow {
    id: i64,
    run_id: i32,
    worker_id: String,
    metrics: JsonValue,
    runtime_metrics: JsonValue,
    engine_diagnostics: JsonValue,
    created_at: DateTime<Utc>,
}

impl From<SamplerPerformanceHistoryRow> for SamplerPerformanceHistoryEntry {
    fn from(value: SamplerPerformanceHistoryRow) -> Self {
        Self {
            id: value.id,
            run_id: value.run_id,
            worker_id: value.worker_id,
            metrics: serde_json::from_value::<SamplerPerformanceMetrics>(value.metrics).unwrap_or(
                SamplerPerformanceMetrics {
                    produced_batches: 0,
                    produced_samples: 0,
                    avg_produce_time_per_sample_ms: 0.0,
                    std_produce_time_per_sample_ms: 0.0,
                    ingested_batches: 0,
                    ingested_samples: 0,
                    avg_ingest_time_per_sample_ms: 0.0,
                    std_ingest_time_per_sample_ms: 0.0,
                },
            ),
            runtime_metrics: value.runtime_metrics,
            engine_diagnostics: value.engine_diagnostics,
            created_at: value.created_at,
        }
    }
}

pub(crate) async fn health_check(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").fetch_one(pool).await?;
    Ok(())
}

const RUN_PROGRESS_COLUMNS: &str = r#"
    run_id,
    run_name,
    run_status,
    integration_params,
    target,
    evaluator_init_metadata,
    sampler_aggregator_init_metadata,
    started_at,
    completed_at,
    training_completed_at,
    total_batches_planned,
    batches_completed,
    total_batches,
    total_samples,
    pending_batches,
    claimed_batches,
    completed_batches,
    failed_batches,
    completion_rate
"#;

const RUN_BATCH_STATS_SUBQUERY_FOR_ONE_RUN: &str = r#"
    SELECT
        run_id,
        COUNT(*) as total_batches,
        SUM(batch_size) as total_samples,
        SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending_batches,
        SUM(CASE WHEN status = 'claimed' THEN 1 ELSE 0 END) as claimed_batches,
        SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed_batches,
        SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed_batches
    FROM batches
    WHERE run_id = $1
    GROUP BY run_id
"#;

pub(crate) async fn get_all_runs(pool: &PgPool) -> Result<Vec<RunProgress>, sqlx::Error> {
    let sql = format!(
        r#"
        SELECT
            {columns}
        FROM run_progress
        ORDER BY started_at DESC
        "#,
        columns = RUN_PROGRESS_COLUMNS
    );

    let rows = sqlx::query_as::<_, RunProgressRow>(&sql)
        .fetch_all(pool)
        .await?;

    rows.into_iter().map(TryInto::try_into).collect()
}

pub(crate) async fn get_run_progress(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<RunProgress>, sqlx::Error> {
    let sql = format!(
        r#"
        WITH run_progress AS (
            SELECT
                r.id as run_id,
                r.name as run_name,
                r.status as run_status,
                (
                    COALESCE(r.integration_params, '{{}}'::jsonb)
                    || jsonb_build_object('observable_implementation', r.observable_implementation)
                ) as integration_params,
                r.target,
                r.evaluator_init_metadata,
                r.sampler_aggregator_init_metadata,
                r.started_at,
                r.completed_at,
                r.training_completed_at,
                r.total_batches_planned,
                r.batches_completed,
                COALESCE(b.total_batches, 0) as total_batches,
                COALESCE(b.total_samples, 0) as total_samples,
                COALESCE(b.pending_batches, 0) as pending_batches,
                COALESCE(b.claimed_batches, 0) as claimed_batches,
                COALESCE(b.completed_batches, 0) as completed_batches,
                COALESCE(b.failed_batches, 0) as failed_batches,
                CASE
                    WHEN COALESCE(b.total_batches, 0) > 0
                    THEN CAST(COALESCE(b.completed_batches, 0) AS FLOAT) / b.total_batches
                    ELSE 0.0
                END as completion_rate
            FROM runs r
            LEFT JOIN (
                {batch_stats_subquery}
            ) b ON r.id = b.run_id
            WHERE r.id = $1
        )
        SELECT
            {columns}
        FROM run_progress
        "#,
        columns = RUN_PROGRESS_COLUMNS,
        batch_stats_subquery = RUN_BATCH_STATS_SUBQUERY_FOR_ONE_RUN
    );

    let row = sqlx::query_as::<_, RunProgressRow>(&sql)
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

pub(crate) async fn get_worker_logs(
    pool: &PgPool,
    run_id: i32,
    limit: i64,
    worker_id: Option<&str>,
    level: Option<&str>,
    after_id: Option<i64>,
) -> Result<Vec<WorkerLogEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, WorkerLogRow>(
        r#"
        SELECT
            id,
            ts,
            run_id,
            node_id,
            worker_id,
            role,
            level,
            event_type,
            message,
            fields
        FROM (
            SELECT
                id,
                ts,
                run_id,
                node_id,
                worker_id,
                role,
                level,
                event_type,
                message,
                fields
            FROM worker_logs
            WHERE run_id = $1
              AND ($2::text IS NULL OR worker_id = $2)
              AND ($3::text IS NULL OR level = $3)
              AND ($4::bigint IS NULL OR id > $4)
            ORDER BY id DESC
            LIMIT $5
        ) recent
        ORDER BY id ASC
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .bind(level)
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub(crate) async fn get_registered_workers(
    pool: &PgPool,
    run_id: Option<i32>,
) -> Result<Vec<RegisteredWorkerEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RegisteredWorkerRow>(
        r#"
        SELECT
            w.worker_id,
            w.node_id,
            w.role,
            w.implementation,
            w.version,
            w.status,
            w.last_seen,
            e.metrics AS evaluator_metrics,
            p.metrics AS sampler_metrics,
            p.runtime_metrics AS sampler_runtime_metrics,
            p.engine_diagnostics AS sampler_engine_diagnostics,
            e.engine_diagnostics AS evaluator_engine_diagnostics
        FROM workers w
        LEFT JOIN sampler_aggregator_performance_latest p
            ON p.run_id = COALESCE($1, w.desired_run_id)
           AND p.worker_id = w.worker_id
        LEFT JOIN evaluator_performance_latest e
            ON e.run_id = COALESCE($1, w.desired_run_id)
           AND e.worker_id = w.worker_id
        WHERE ($1::int IS NULL OR w.desired_run_id = $1)
        ORDER BY
            CASE w.status
                WHEN 'active' THEN 0
                WHEN 'draining' THEN 1
                ELSE 2
            END,
            w.last_seen DESC NULLS LAST,
            w.worker_id ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub(crate) async fn get_evaluator_performance_history(
    pool: &PgPool,
    run_id: i32,
    limit: i64,
    worker_id: Option<&str>,
) -> Result<Vec<EvaluatorPerformanceHistoryEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EvaluatorPerformanceHistoryRow>(
        r#"
        SELECT
            id,
            run_id,
            worker_id,
            metrics,
            engine_diagnostics,
            created_at
        FROM evaluator_performance_history
        WHERE run_id = $1
          AND ($2::text IS NULL OR worker_id = $2)
        ORDER BY created_at DESC, id DESC
        LIMIT $3
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub(crate) async fn get_sampler_performance_history(
    pool: &PgPool,
    run_id: i32,
    limit: i64,
    worker_id: Option<&str>,
) -> Result<Vec<SamplerPerformanceHistoryEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, SamplerPerformanceHistoryRow>(
        r#"
        SELECT
            id,
            run_id,
            worker_id,
            metrics,
            runtime_metrics,
            engine_diagnostics,
            created_at
        FROM sampler_aggregator_performance_history
        WHERE run_id = $1
          AND ($2::text IS NULL OR worker_id = $2)
        ORDER BY created_at DESC, id DESC
        LIMIT $3
        "#,
    )
    .bind(run_id)
    .bind(worker_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}
