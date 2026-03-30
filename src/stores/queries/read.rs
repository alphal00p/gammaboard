use crate::core::SamplerPerformanceMetrics;
use crate::evaluation::ObservableState;
use crate::stores::{
    EvaluatorPerformanceHistoryEntry, RegisteredWorkerEntry, RunProgress,
    SamplerPerformanceHistoryEntry, TaskOutputSnapshot, TaskStageSnapshot, WorkQueueStats,
    WorkerLogEntry, WorkerLogPage,
};
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{fmt::Display, io};

fn invalid_data_error(context: &str, err: impl Display) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{context}: {err}"),
    )))
}

fn decode_json<T: DeserializeOwned>(value: JsonValue, context: &str) -> Result<T, sqlx::Error> {
    serde_json::from_value(value).map_err(|err| invalid_data_error(context, err))
}

fn decode_optional_json<T: DeserializeOwned>(value: Option<JsonValue>) -> Option<T> {
    value.and_then(|payload| serde_json::from_value(payload).ok())
}

fn decode_json_or_default<T: DeserializeOwned + Default>(value: JsonValue) -> T {
    serde_json::from_value(value).unwrap_or_default()
}

fn default_sampler_performance_metrics() -> SamplerPerformanceMetrics {
    SamplerPerformanceMetrics {
        produced_batches: 0,
        produced_samples: 0,
        avg_produce_time_per_sample_ms: 0.0,
        std_produce_time_per_sample_ms: 0.0,
        ingested_batches: 0,
        ingested_samples: 0,
        avg_ingest_time_per_sample_ms: 0.0,
        std_ingest_time_per_sample_ms: 0.0,
    }
}

fn id_text(value: impl Display) -> String {
    value.to_string()
}

#[derive(sqlx::FromRow)]
struct RunProgressRow {
    run_id: i32,
    run_name: String,
    root_stage_snapshot_id: Option<i64>,
    lifecycle_state: String,
    desired_assignment_count: i64,
    active_worker_count: i64,
    integration_params: Option<JsonValue>,
    domain: Option<JsonValue>,
    active_task_id: Option<i64>,
    target: Option<JsonValue>,
    nr_produced_samples: i64,
    nr_completed_samples: i64,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
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
            root_stage_snapshot_id: value.root_stage_snapshot_id.map(id_text),
            lifecycle_state: value.lifecycle_state,
            desired_assignment_count: value.desired_assignment_count,
            active_worker_count: value.active_worker_count,
            integration_params: value.integration_params,
            domain: value
                .domain
                .map(|payload| decode_json(payload, "invalid domain payload"))
                .transpose()?,
            active_task_id: value.active_task_id.map(id_text),
            target: value.target,
            nr_produced_samples: value.nr_produced_samples,
            nr_completed_samples: value.nr_completed_samples,
            started_at: value.started_at,
            completed_at: value.completed_at,
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
struct TaskOutputSnapshotRow {
    id: i64,
    run_id: i32,
    task_id: i64,
    persisted_output: JsonValue,
    created_at: Option<DateTime<Utc>>,
}

impl From<TaskOutputSnapshotRow> for TaskOutputSnapshot {
    fn from(value: TaskOutputSnapshotRow) -> Self {
        Self {
            id: id_text(value.id),
            run_id: value.run_id,
            task_id: id_text(value.task_id),
            persisted_output: value.persisted_output,
            created_at: value.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct TaskStageSnapshotRow {
    id: i64,
    run_id: i32,
    task_id: i64,
    observable_state: JsonValue,
    created_at: Option<DateTime<Utc>>,
}

impl TryFrom<TaskStageSnapshotRow> for TaskStageSnapshot {
    type Error = sqlx::Error;

    fn try_from(value: TaskStageSnapshotRow) -> Result<Self, Self::Error> {
        let observable_state =
            ObservableState::from_json(&value.observable_state).map_err(|err| {
                invalid_data_error(
                    "failed to decode observable_state from run_stage_snapshots",
                    err,
                )
            })?;
        Ok(Self {
            id: id_text(value.id),
            run_id: value.run_id,
            task_id: id_text(value.task_id),
            observable_state,
            created_at: value.created_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct WorkerLogRow {
    id: i64,
    ts: DateTime<Utc>,
    run_id: Option<i32>,
    node_uuid: Option<String>,
    node_name: Option<String>,
    level: String,
    message: String,
    fields: JsonValue,
}

impl From<WorkerLogRow> for WorkerLogEntry {
    fn from(value: WorkerLogRow) -> Self {
        Self {
            id: id_text(value.id),
            ts: value.ts,
            run_id: value.run_id,
            node_uuid: value.node_uuid,
            node_name: value.node_name,
            level: value.level,
            message: value.message,
            fields: value.fields,
        }
    }
}

#[derive(sqlx::FromRow)]
struct RegisteredWorkerRow {
    node_name: String,
    node_uuid: String,
    desired_run_id: Option<i32>,
    desired_run_name: Option<String>,
    desired_role: Option<String>,
    current_run_id: Option<i32>,
    current_run_name: Option<String>,
    current_role: Option<String>,
    role: String,
    implementation: String,
    version: String,
    status: String,
    last_seen: Option<DateTime<Utc>>,
    evaluator_metrics: Option<JsonValue>,
    sampler_metrics: Option<JsonValue>,
    sampler_runtime_metrics: Option<JsonValue>,
    sampler_engine_diagnostics: Option<JsonValue>,
}

impl From<RegisteredWorkerRow> for RegisteredWorkerEntry {
    fn from(value: RegisteredWorkerRow) -> Self {
        Self {
            node_name: value.node_name,
            node_uuid: value.node_uuid,
            desired_run_id: value.desired_run_id,
            desired_run_name: value.desired_run_name,
            desired_role: value.desired_role,
            current_run_id: value.current_run_id,
            current_run_name: value.current_run_name,
            current_role: value.current_role,
            role: value.role,
            implementation: value.implementation,
            version: value.version,
            status: value.status,
            last_seen: value.last_seen,
            evaluator_metrics: decode_optional_json(value.evaluator_metrics),
            sampler_metrics: decode_optional_json(value.sampler_metrics),
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
    created_at: DateTime<Utc>,
}

impl From<EvaluatorPerformanceHistoryRow> for EvaluatorPerformanceHistoryEntry {
    fn from(value: EvaluatorPerformanceHistoryRow) -> Self {
        Self {
            id: value.id,
            run_id: value.run_id,
            worker_id: value.worker_id,
            metrics: decode_json_or_default(value.metrics),
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
            metrics: serde_json::from_value(value.metrics)
                .unwrap_or_else(|_| default_sampler_performance_metrics()),
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

const RUN_ASSIGNMENT_STATS_SUBQUERY: &str = r#"
    SELECT
        r.id AS run_id,
        COALESCE(da.desired_assignment_count, 0) AS desired_assignment_count,
        COALESCE(aw.active_worker_count, 0) AS active_worker_count
    FROM runs r
    LEFT JOIN (
        SELECT desired_run_id AS run_id, COUNT(*) AS desired_assignment_count
        FROM nodes
        WHERE desired_run_id IS NOT NULL
        GROUP BY desired_run_id
    ) da ON r.id = da.run_id
    LEFT JOIN (
        SELECT active_run_id AS run_id, COUNT(*) AS active_worker_count
        FROM nodes
        WHERE active_run_id IS NOT NULL
        GROUP BY active_run_id
    ) aw ON r.id = aw.run_id
"#;

const RUN_ROOT_STAGE_SNAPSHOT_SUBQUERY: &str = r#"
    SELECT
        run_id,
        id AS root_stage_snapshot_id
    FROM run_stage_snapshots
    WHERE queue_empty = TRUE
      AND task_id IS NULL
      AND sequence_nr = 0
"#;

const RUN_BATCH_STATS_SUBQUERY_ALL_RUNS: &str = r#"
    SELECT
        run_id,
        COUNT(*) as total_batches,
        SUM(batch_size) as total_samples,
        SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending_batches,
        SUM(CASE WHEN status = 'claimed' THEN 1 ELSE 0 END) as claimed_batches,
        SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) as completed_batches,
        SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed_batches
    FROM batches
    GROUP BY run_id
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

fn run_progress_sql(batch_stats_subquery: &str, run_where_clause: &str) -> String {
    format!(
        r#"
        WITH assignment_stats AS (
            {assignment_stats_subquery}
        )
        SELECT
            r.id as run_id,
            r.name as run_name,
            root.root_stage_snapshot_id,
            CASE
                WHEN COALESCE(a.desired_assignment_count, 0) > 0 THEN 'running'
                WHEN COALESCE(b.claimed_batches, 0) > 0 OR COALESCE(a.active_worker_count, 0) > 0 THEN 'pausing'
                ELSE 'paused'
            END as lifecycle_state,
            COALESCE(a.desired_assignment_count, 0) as desired_assignment_count,
            COALESCE(a.active_worker_count, 0) as active_worker_count,
            COALESCE(r.integration_params, '{{}}'::jsonb) as integration_params,
            r.point_spec as domain,
            active_task.id as active_task_id,
            r.target,
            r.nr_produced_samples,
            r.nr_completed_samples,
            r.started_at,
            r.completed_at,
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
        LEFT JOIN assignment_stats a ON r.id = a.run_id
        LEFT JOIN (
            {root_stage_snapshot_subquery}
        ) root ON r.id = root.run_id
        LEFT JOIN run_tasks active_task
            ON active_task.run_id = r.id
           AND active_task.state = 'active'
        {run_where_clause}
        "#,
        assignment_stats_subquery = RUN_ASSIGNMENT_STATS_SUBQUERY,
        batch_stats_subquery = batch_stats_subquery,
        root_stage_snapshot_subquery = RUN_ROOT_STAGE_SNAPSHOT_SUBQUERY,
        run_where_clause = run_where_clause
    )
}

pub(crate) async fn get_all_runs(pool: &PgPool) -> Result<Vec<RunProgress>, sqlx::Error> {
    let mut sql = run_progress_sql(RUN_BATCH_STATS_SUBQUERY_ALL_RUNS, "");
    sql.push_str("\nORDER BY started_at DESC");

    let rows = sqlx::query_as::<_, RunProgressRow>(&sql)
        .fetch_all(pool)
        .await?;

    rows.into_iter().map(TryInto::try_into).collect()
}

pub(crate) async fn get_run_progress(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<RunProgress>, sqlx::Error> {
    let sql = run_progress_sql(RUN_BATCH_STATS_SUBQUERY_FOR_ONE_RUN, "WHERE r.id = $1");

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

pub(crate) async fn get_task_output_snapshots(
    pool: &PgPool,
    run_id: i32,
    task_id: i64,
    after_snapshot_id: Option<i64>,
    limit: i64,
) -> Result<Vec<TaskOutputSnapshot>, sqlx::Error> {
    let rows = sqlx::query_as::<_, TaskOutputSnapshotRow>(
        r#"
        SELECT
            id,
            run_id,
            task_id,
            persisted_observable AS persisted_output,
            created_at
        FROM persisted_observable_snapshots
        WHERE run_id = $1
          AND task_id = $2
          AND ($3::bigint IS NULL OR id > $3)
        ORDER BY id DESC
        LIMIT $4
        "#,
    )
    .bind(run_id)
    .bind(task_id)
    .bind(after_snapshot_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub(crate) async fn get_latest_task_stage_snapshot(
    pool: &PgPool,
    run_id: i32,
    task_id: i64,
) -> Result<Option<TaskStageSnapshot>, sqlx::Error> {
    let row = sqlx::query_as::<_, TaskStageSnapshotRow>(
        r#"
        SELECT
            id,
            run_id,
            task_id,
            observable_state,
            created_at
        FROM run_stage_snapshots
        WHERE run_id = $1
          AND task_id = $2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    row.map(TryInto::try_into).transpose()
}

pub(crate) async fn get_worker_logs(
    pool: &PgPool,
    run_id: i32,
    limit: i64,
    node_name: Option<&str>,
    level: Option<&str>,
    query: Option<&str>,
    before_id: Option<i64>,
) -> Result<WorkerLogPage, sqlx::Error> {
    let query_pattern = query.map(|value| format!("%{value}%"));
    let rows = sqlx::query_as::<_, WorkerLogRow>(
        r#"
        SELECT
            id,
            ts,
            run_id,
            node_uuid,
            node_name,
            level,
            message,
            fields
        FROM (
            SELECT
                id,
                ts,
                run_id,
                node_uuid,
                node_name,
                level,
                message,
                fields
            FROM runtime_logs
            WHERE source = 'worker'
              AND run_id = $1
              AND ($2::text IS NULL OR node_name = $2)
              AND ($3::text IS NULL OR level = $3)
              AND ($4::text IS NULL OR message ILIKE $4 OR fields::text ILIKE $4)
              AND ($5::bigint IS NULL OR id < $5)
            ORDER BY id DESC
            LIMIT $6
        ) recent
        ORDER BY id DESC
        "#,
    )
    .bind(run_id)
    .bind(node_name)
    .bind(level)
    .bind(query_pattern)
    .bind(before_id)
    .bind(limit + 1)
    .fetch_all(pool)
    .await?;

    let has_more_older = rows.len() as i64 > limit;
    let items: Vec<WorkerLogEntry> = rows
        .into_iter()
        .take(limit as usize)
        .map(Into::into)
        .collect();
    let next_before_id = if has_more_older {
        items.last().map(|entry| entry.id.clone())
    } else {
        None
    };

    Ok(WorkerLogPage {
        items,
        next_before_id,
        has_more_older,
    })
}

pub(crate) async fn get_registered_workers(
    pool: &PgPool,
    run_id: Option<i32>,
) -> Result<Vec<RegisteredWorkerEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RegisteredWorkerRow>(
        r#"
        SELECT
            n.name AS node_name,
            n.uuid AS node_uuid,
            n.desired_run_id,
            dr.name AS desired_run_name,
            n.desired_role,
            n.active_run_id AS current_run_id,
            cr.name AS current_run_name,
            n.active_role AS current_role,
            COALESCE(n.active_role, n.desired_role, 'none') AS role,
            'run_node' AS implementation,
            'node' AS version,
            CASE
                WHEN n.active_role IS NOT NULL THEN 'active'
                ELSE 'inactive'
            END AS status,
            n.last_seen,
            e.metrics AS evaluator_metrics,
            p.metrics AS sampler_metrics,
            p.runtime_metrics AS sampler_runtime_metrics,
            p.engine_diagnostics AS sampler_engine_diagnostics
        FROM nodes n
        LEFT JOIN sampler_aggregator_performance_latest p
            ON p.run_id = COALESCE($1, n.active_run_id, n.desired_run_id)
           AND p.worker_id = n.name
        LEFT JOIN evaluator_performance_latest e
            ON e.run_id = COALESCE($1, n.active_run_id, n.desired_run_id)
           AND e.worker_id = n.name
        LEFT JOIN runs dr ON dr.id = n.desired_run_id
        LEFT JOIN runs cr ON cr.id = n.active_run_id
        WHERE n.lease_expires_at > now()
          AND ($1::int IS NULL OR n.desired_run_id = $1 OR n.active_run_id = $1)
        ORDER BY
            CASE
                WHEN n.active_role IS NOT NULL THEN 0
                ELSE 1
            END,
            n.last_seen DESC NULLS LAST,
            n.name ASC
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

pub(crate) async fn get_worker_evaluator_performance_history(
    pool: &PgPool,
    worker_id: &str,
    limit: i64,
) -> Result<Vec<EvaluatorPerformanceHistoryEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EvaluatorPerformanceHistoryRow>(
        r#"
        SELECT
            id,
            run_id,
            worker_id,
            metrics,
            created_at
        FROM evaluator_performance_history
        WHERE worker_id = $1
        ORDER BY created_at DESC, id DESC
        LIMIT $2
        "#,
    )
    .bind(worker_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub(crate) async fn get_worker_sampler_performance_history(
    pool: &PgPool,
    worker_id: &str,
    limit: i64,
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
        WHERE worker_id = $1
        ORDER BY created_at DESC, id DESC
        LIMIT $2
        "#,
    )
    .bind(worker_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}
