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
use std::{collections::HashMap, fmt::Display, io};

fn invalid_data_error(context: &str, err: impl Display) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{context}: {err}"),
    )))
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
struct RunProgressBaseRow {
    run_id: i32,
    run_name: String,
    root_stage_snapshot_id: Option<i64>,
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
}

#[derive(Debug, Clone, Copy, Default)]
struct BatchStats {
    total_batches: i64,
    total_samples: i64,
    pending_batches: i64,
    claimed_batches: i64,
    completed_batches: i64,
    failed_batches: i64,
}

impl RunProgressBaseRow {
    fn into_run_progress(self, batch_stats: BatchStats) -> RunProgress {
        let completion_rate = if batch_stats.total_batches > 0 {
            batch_stats.completed_batches as f64 / batch_stats.total_batches as f64
        } else {
            0.0
        };
        let lifecycle_state = if self.desired_assignment_count > 0 {
            "running"
        } else if batch_stats.claimed_batches > 0 || self.active_worker_count > 0 {
            "pausing"
        } else {
            "paused"
        }
        .to_string();
        RunProgress {
            run_id: self.run_id,
            run_name: self.run_name,
            root_stage_snapshot_id: self.root_stage_snapshot_id.map(id_text),
            lifecycle_state,
            desired_assignment_count: self.desired_assignment_count,
            active_worker_count: self.active_worker_count,
            integration_params: self.integration_params,
            domain: decode_optional_json(self.domain),
            active_task_id: self.active_task_id.map(id_text),
            target: self.target,
            nr_produced_samples: self.nr_produced_samples,
            nr_completed_samples: self.nr_completed_samples,
            started_at: self.started_at,
            completed_at: self.completed_at,
            batches_completed: self.batches_completed,
            total_batches: batch_stats.total_batches,
            total_samples: batch_stats.total_samples,
            pending_batches: batch_stats.pending_batches,
            claimed_batches: batch_stats.claimed_batches,
            completed_batches: batch_stats.completed_batches,
            failed_batches: batch_stats.failed_batches,
            completion_rate,
        }
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
    evaluator_rss_bytes: Option<i64>,
    sampler_metrics: Option<JsonValue>,
    sampler_runtime_metrics: Option<JsonValue>,
    sampler_engine_diagnostics: Option<JsonValue>,
    sampler_rss_bytes: Option<i64>,
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
            evaluator_rss_bytes: value.evaluator_rss_bytes,
            sampler_metrics: decode_optional_json(value.sampler_metrics),
            sampler_runtime_metrics: value.sampler_runtime_metrics,
            sampler_engine_diagnostics: value.sampler_engine_diagnostics,
            sampler_rss_bytes: value.sampler_rss_bytes,
        }
    }
}

#[derive(sqlx::FromRow)]
struct EvaluatorPerformanceHistoryRow {
    id: i64,
    run_id: i32,
    worker_id: String,
    metrics: JsonValue,
    rss_bytes: Option<i64>,
    created_at: DateTime<Utc>,
}

impl From<EvaluatorPerformanceHistoryRow> for EvaluatorPerformanceHistoryEntry {
    fn from(value: EvaluatorPerformanceHistoryRow) -> Self {
        Self {
            id: value.id,
            run_id: value.run_id,
            worker_id: value.worker_id,
            metrics: decode_json_or_default(value.metrics),
            rss_bytes: value.rss_bytes,
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
    rss_bytes: Option<i64>,
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
            rss_bytes: value.rss_bytes,
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

fn run_progress_sql(run_where_clause: &str) -> String {
    format!(
        r#"
        WITH assignment_stats AS (
            {assignment_stats_subquery}
        )
        SELECT
            r.id as run_id,
            r.name as run_name,
            root.root_stage_snapshot_id,
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
            r.batches_completed
        FROM runs r
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
        root_stage_snapshot_subquery = RUN_ROOT_STAGE_SNAPSHOT_SUBQUERY,
        run_where_clause = run_where_clause
    )
}

async fn load_batch_stats_for_runs(
    pool: &PgPool,
    run_ids: &[i32],
) -> Result<HashMap<i32, BatchStats>, sqlx::Error> {
    if run_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = sqlx::query_as::<_, (i32, i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            run_id,
            COUNT(*) AS total_batches,
            COALESCE(SUM(batch_size), 0) AS total_samples,
            COUNT(*) FILTER (WHERE status = 'pending') AS pending_batches,
            COUNT(*) FILTER (WHERE status = 'claimed') AS claimed_batches,
            COUNT(*) FILTER (WHERE status = 'completed') AS completed_batches,
            COUNT(*) FILTER (WHERE status = 'failed') AS failed_batches
        FROM batches
        WHERE run_id = ANY($1)
        GROUP BY run_id
        "#,
    )
    .bind(run_ids)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                run_id,
                total_batches,
                total_samples,
                pending_batches,
                claimed_batches,
                completed_batches,
                failed_batches,
            )| {
                (
                    run_id,
                    BatchStats {
                        total_batches,
                        total_samples,
                        pending_batches,
                        claimed_batches,
                        completed_batches,
                        failed_batches,
                    },
                )
            },
        )
        .collect())
}

pub(crate) async fn get_all_runs(pool: &PgPool) -> Result<Vec<RunProgress>, sqlx::Error> {
    let mut sql = run_progress_sql("");
    sql.push_str("\nORDER BY started_at DESC");

    let rows = sqlx::query_as::<_, RunProgressBaseRow>(&sql)
        .fetch_all(pool)
        .await?;
    let run_ids = rows.iter().map(|row| row.run_id).collect::<Vec<_>>();
    let batch_stats = load_batch_stats_for_runs(pool, &run_ids).await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let stats = batch_stats.get(&row.run_id).copied().unwrap_or_default();
            row.into_run_progress(stats)
        })
        .collect())
}

pub(crate) async fn get_run_progress(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<RunProgress>, sqlx::Error> {
    let sql = run_progress_sql("WHERE r.id = $1");

    let row = sqlx::query_as::<_, RunProgressBaseRow>(&sql)
        .bind(run_id)
        .fetch_optional(pool)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let batch_stats = load_batch_stats_for_runs(pool, &[run_id]).await?;
    Ok(Some(row.into_run_progress(
        batch_stats.get(&run_id).copied().unwrap_or_default(),
    )))
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
    let rows = match run_id {
        Some(run_id) => {
            sqlx::query_as::<_, RegisteredWorkerRow>(
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
                    e.rss_bytes AS evaluator_rss_bytes,
                    p.metrics AS sampler_metrics,
                    p.runtime_metrics AS sampler_runtime_metrics,
                    p.engine_diagnostics AS sampler_engine_diagnostics,
                    p.rss_bytes AS sampler_rss_bytes
                FROM nodes n
                LEFT JOIN sampler_aggregator_performance_latest p
                    ON p.run_id = $1
                   AND p.worker_id = n.name
                LEFT JOIN evaluator_performance_latest e
                    ON e.run_id = $1
                   AND e.worker_id = n.name
                LEFT JOIN runs dr ON dr.id = n.desired_run_id
                LEFT JOIN runs cr ON cr.id = n.active_run_id
                WHERE n.lease_expires_at > now()
                  AND (n.desired_run_id = $1 OR n.active_run_id = $1)
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
            .await?
        }
        None => {
            sqlx::query_as::<_, RegisteredWorkerRow>(
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
                    e.rss_bytes AS evaluator_rss_bytes,
                    p.metrics AS sampler_metrics,
                    p.runtime_metrics AS sampler_runtime_metrics,
                    p.engine_diagnostics AS sampler_engine_diagnostics,
                    p.rss_bytes AS sampler_rss_bytes
                FROM nodes n
                LEFT JOIN sampler_aggregator_performance_latest p
                    ON p.run_id = COALESCE(n.active_run_id, n.desired_run_id)
                   AND p.worker_id = n.name
                LEFT JOIN evaluator_performance_latest e
                    ON e.run_id = COALESCE(n.active_run_id, n.desired_run_id)
                   AND e.worker_id = n.name
                LEFT JOIN runs dr ON dr.id = n.desired_run_id
                LEFT JOIN runs cr ON cr.id = n.active_run_id
                WHERE n.lease_expires_at > now()
                ORDER BY
                    CASE
                        WHEN n.active_role IS NOT NULL THEN 0
                        ELSE 1
                    END,
                    n.last_seen DESC NULLS LAST,
                    n.name ASC
                "#,
            )
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows.into_iter().map(Into::into).collect())
}

pub(crate) async fn get_registered_worker_summaries(
    pool: &PgPool,
    run_id: Option<i32>,
) -> Result<Vec<RegisteredWorkerEntry>, sqlx::Error> {
    let rows = match run_id {
        Some(run_id) => {
            sqlx::query_as::<_, RegisteredWorkerRow>(
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
                    NULL::jsonb AS evaluator_metrics,
                    NULL::bigint AS evaluator_rss_bytes,
                    NULL::jsonb AS sampler_metrics,
                    NULL::jsonb AS sampler_runtime_metrics,
                    NULL::jsonb AS sampler_engine_diagnostics,
                    NULL::bigint AS sampler_rss_bytes
                FROM nodes n
                LEFT JOIN runs dr ON dr.id = n.desired_run_id
                LEFT JOIN runs cr ON cr.id = n.active_run_id
                WHERE n.lease_expires_at > now()
                  AND (n.desired_run_id = $1 OR n.active_run_id = $1)
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
            .await?
        }
        None => {
            sqlx::query_as::<_, RegisteredWorkerRow>(
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
                    NULL::jsonb AS evaluator_metrics,
                    NULL::bigint AS evaluator_rss_bytes,
                    NULL::jsonb AS sampler_metrics,
                    NULL::jsonb AS sampler_runtime_metrics,
                    NULL::jsonb AS sampler_engine_diagnostics,
                    NULL::bigint AS sampler_rss_bytes
                FROM nodes n
                LEFT JOIN runs dr ON dr.id = n.desired_run_id
                LEFT JOIN runs cr ON cr.id = n.active_run_id
                WHERE n.lease_expires_at > now()
                ORDER BY
                    CASE
                        WHEN n.active_role IS NOT NULL THEN 0
                        ELSE 1
                    END,
                    n.last_seen DESC NULLS LAST,
                    n.name ASC
                "#,
            )
            .fetch_all(pool)
            .await?
        }
    };

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
            rss_bytes,
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
            rss_bytes,
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
            rss_bytes,
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
            rss_bytes,
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
