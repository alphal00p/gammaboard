use crate::core::{SamplerPerformanceMetrics, WorkerRole};
use crate::evaluation::AccumulatorState;
use crate::stores::{
    EvaluatorPerformanceHistoryEntry, RegisteredWorkerEntry, RegisteredWorkerSummary,
    RunLifecycleState, RunProgress, RuntimeLogEntry, RuntimeLogPage,
    SamplerPerformanceHistoryEntry, TaskOutputSnapshot, TaskStageSnapshot, WorkerStatus,
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

fn parse_worker_role(role: Option<String>) -> Option<WorkerRole> {
    role.and_then(|role| role.parse().ok())
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
        completed_samples_total: 0,
        sampler_uptime_ms: 0.0,
    }
}

fn id_text(value: impl Display) -> String {
    value.to_string()
}

#[derive(sqlx::FromRow)]
struct RunProgressBaseRow {
    run_id: i32,
    run_name: String,
    run_toml: Option<String>,
    provenance: JsonValue,
    parent_run_id: Option<i32>,
    parent_task_id: Option<i64>,
    spawn_kind: Option<String>,
    spawn_label: Option<String>,
    root_stage_snapshot_id: Option<i64>,
    desired_assignment_count: i64,
    active_worker_count: i64,
    integration_params: Option<JsonValue>,
    domain: Option<JsonValue>,
    active_task_id: Option<i64>,
    target: Option<JsonValue>,
    nr_produced_samples: i64,
    nr_completed_samples: i64,
    sampler_runner_uptime_ms: f64,
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
        let lifecycle_state = if self.desired_assignment_count > 0 || self.active_task_id.is_some()
        {
            RunLifecycleState::Running
        } else if batch_stats.claimed_batches > 0 || self.active_worker_count > 0 {
            RunLifecycleState::Pausing
        } else {
            RunLifecycleState::Paused
        };
        RunProgress {
            run_id: self.run_id,
            run_name: self.run_name,
            run_toml: self.run_toml,
            provenance: self.provenance,
            parent_run_id: self.parent_run_id,
            parent_task_id: self.parent_task_id.map(id_text),
            spawn_kind: self.spawn_kind,
            spawn_label: self.spawn_label,
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
            nr_produced_samples_including_children: self.nr_produced_samples,
            nr_completed_samples_including_children: self.nr_completed_samples,
            sampler_runner_uptime_ms: self.sampler_runner_uptime_ms,
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
            AccumulatorState::from_json(&value.observable_state).map_err(|err| {
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
    source: String,
    run_id: Option<i32>,
    node_uuid: Option<String>,
    node_name: Option<String>,
    level: String,
    target: String,
    message: String,
    fields: JsonValue,
}

impl From<WorkerLogRow> for RuntimeLogEntry {
    fn from(value: WorkerLogRow) -> Self {
        Self {
            id: id_text(value.id),
            ts: value.ts,
            source: value.source,
            run_id: value.run_id,
            node_uuid: value.node_uuid,
            node_name: value.node_name,
            level: value.level,
            target: value.target,
            message: value.message,
            fields: value.fields,
        }
    }
}

#[derive(sqlx::FromRow)]
struct RegisteredWorkerRow {
    node_name: String,
    node_uuid: String,
    capabilities: JsonValue,
    desired_run_id: Option<i32>,
    desired_run_name: Option<String>,
    desired_role: Option<String>,
    current_run_id: Option<i32>,
    current_run_name: Option<String>,
    current_role: Option<String>,
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
        let status = if value.current_role.is_some() {
            WorkerStatus::Active
        } else {
            WorkerStatus::Inactive
        };
        Self {
            node_name: value.node_name,
            node_uuid: value.node_uuid,
            capabilities: value.capabilities,
            desired_run_id: value.desired_run_id,
            desired_run_name: value.desired_run_name,
            desired_role: parse_worker_role(value.desired_role),
            current_run_id: value.current_run_id,
            current_run_name: value.current_run_name,
            current_role: parse_worker_role(value.current_role),
            status,
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
struct RegisteredWorkerSummaryRow {
    node_name: String,
    node_uuid: String,
    capabilities: JsonValue,
    desired_run_id: Option<i32>,
    desired_run_name: Option<String>,
    desired_role: Option<String>,
    current_run_id: Option<i32>,
    current_run_name: Option<String>,
    current_role: Option<String>,
    last_seen: Option<DateTime<Utc>>,
}

impl From<RegisteredWorkerSummaryRow> for RegisteredWorkerSummary {
    fn from(value: RegisteredWorkerSummaryRow) -> Self {
        let status = if value.current_role.is_some() {
            WorkerStatus::Active
        } else {
            WorkerStatus::Inactive
        };
        Self {
            node_name: value.node_name,
            node_uuid: value.node_uuid,
            capabilities: value.capabilities,
            desired_run_id: value.desired_run_id,
            desired_run_name: value.desired_run_name,
            desired_role: parse_worker_role(value.desired_role),
            current_run_id: value.current_run_id,
            current_run_name: value.current_run_name,
            current_role: parse_worker_role(value.current_role),
            status,
            last_seen: value.last_seen,
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
            r.run_toml,
            r.provenance,
            r.parent_run_id,
            r.parent_task_id,
            r.spawn_kind,
            r.spawn_label,
            root.root_stage_snapshot_id,
            COALESCE(a.desired_assignment_count, 0) as desired_assignment_count,
            COALESCE(a.active_worker_count, 0) as active_worker_count,
            COALESCE(r.integration_params, '{{}}'::jsonb) as integration_params,
            r.point_spec as domain,
            active_task.id as active_task_id,
            r.target,
            r.nr_produced_samples,
            r.nr_completed_samples,
            r.sampler_runner_uptime_ms,
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
            total_batches,
            total_samples,
            pending_batches,
            claimed_batches,
            completed_batches,
            failed_batches
        FROM run_batch_queue_counters
        WHERE run_id = ANY($1)
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

    let mut runs = rows
        .into_iter()
        .map(|row| {
            let stats = batch_stats.get(&row.run_id).copied().unwrap_or_default();
            row.into_run_progress(stats)
        })
        .collect::<Vec<_>>();
    apply_child_run_sample_totals(&mut runs);
    Ok(runs)
}

pub(crate) async fn get_run_progress(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<RunProgress>, sqlx::Error> {
    Ok(load_run_progress_for_targets(pool, &[run_id]).await?.pop())
}

pub(crate) async fn get_runs_by_name(
    pool: &PgPool,
    run_name: &str,
) -> Result<Vec<RunProgress>, sqlx::Error> {
    let run_ids = sqlx::query_scalar::<_, i32>(
        "SELECT id FROM runs WHERE name = $1 ORDER BY started_at DESC, id DESC",
    )
    .bind(run_name)
    .fetch_all(pool)
    .await?;
    load_run_progress_for_targets(pool, &run_ids).await
}

pub(crate) async fn get_runs_page(
    pool: &PgPool,
    limit: usize,
    offset: usize,
    include_children: bool,
) -> Result<Vec<RunProgress>, sqlx::Error> {
    let run_ids = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT id
        FROM runs
        WHERE $1 OR parent_run_id IS NULL
        ORDER BY started_at DESC, id DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(include_children)
    .bind(limit as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await?;
    load_run_progress_for_targets(pool, &run_ids).await
}

pub(crate) async fn get_child_runs_for_task(
    pool: &PgPool,
    parent_run_id: i32,
    parent_task_id: i64,
    spawn_kind: &str,
) -> Result<Vec<RunProgress>, sqlx::Error> {
    let run_ids = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT id
        FROM runs
        WHERE parent_run_id = $1
          AND parent_task_id = $2
          AND spawn_kind = $3
        ORDER BY started_at DESC, id DESC
        "#,
    )
    .bind(parent_run_id)
    .bind(parent_task_id)
    .bind(spawn_kind)
    .fetch_all(pool)
    .await?;
    load_run_progress_for_targets(pool, &run_ids).await
}

pub(crate) async fn get_control_plane_run_ids(pool: &PgPool) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar::<_, i32>(
        r#"
        SELECT DISTINCT r.id
        FROM runs r
        LEFT JOIN nodes n
          ON n.lease_expires_at > now()
         AND (n.desired_run_id = r.id OR n.active_run_id = r.id)
        LEFT JOIN run_tasks active_task
          ON active_task.run_id = r.id
         AND active_task.state = 'active'
        WHERE n.uuid IS NOT NULL
           OR active_task.task->>'kind' IN ('parameter_scan', 'hyperparameter_tuning', 'integration_campaign', 'set_accumulator')
        ORDER BY r.id
        "#,
    )
    .fetch_all(pool)
    .await
}

async fn load_run_progress_for_targets(
    pool: &PgPool,
    target_ids: &[i32],
) -> Result<Vec<RunProgress>, sqlx::Error> {
    if target_ids.is_empty() {
        return Ok(Vec::new());
    }

    let all_ids = sqlx::query_scalar::<_, i32>(
        r#"
        WITH RECURSIVE descendants(id) AS (
            SELECT UNNEST($1::int[])
            UNION
            SELECT child.id
            FROM runs child
            JOIN descendants parent ON child.parent_run_id = parent.id
        )
        SELECT id FROM descendants
        "#,
    )
    .bind(target_ids)
    .fetch_all(pool)
    .await?;
    let sql = run_progress_sql("WHERE r.id = ANY($1) ORDER BY r.started_at DESC, r.id DESC");
    let rows = sqlx::query_as::<_, RunProgressBaseRow>(&sql)
        .bind(&all_ids)
        .fetch_all(pool)
        .await?;
    let batch_stats = load_batch_stats_for_runs(pool, &all_ids).await?;
    let mut runs = rows
        .into_iter()
        .map(|row| {
            let stats = batch_stats.get(&row.run_id).copied().unwrap_or_default();
            row.into_run_progress(stats)
        })
        .collect::<Vec<_>>();
    apply_child_run_sample_totals(&mut runs);

    let mut by_id = runs
        .into_iter()
        .map(|run| (run.run_id, run))
        .collect::<HashMap<_, _>>();
    Ok(target_ids
        .iter()
        .filter_map(|run_id| by_id.remove(run_id))
        .collect())
}

fn apply_child_run_sample_totals(runs: &mut [RunProgress]) {
    fn accumulate(
        index: usize,
        runs: &[RunProgress],
        children_by_parent: &HashMap<i32, Vec<usize>>,
        totals: &mut HashMap<i32, (i64, i64)>,
    ) -> (i64, i64) {
        let run = &runs[index];
        if let Some(total) = totals.get(&run.run_id) {
            return *total;
        }
        let mut produced = run.nr_produced_samples;
        let mut completed = run.nr_completed_samples;
        if let Some(children) = children_by_parent.get(&run.run_id) {
            for &child_index in children {
                let (child_produced, child_completed) =
                    accumulate(child_index, runs, children_by_parent, totals);
                produced = produced.saturating_add(child_produced);
                completed = completed.saturating_add(child_completed);
            }
        }
        totals.insert(run.run_id, (produced, completed));
        (produced, completed)
    }

    let mut children_by_parent: HashMap<i32, Vec<usize>> = HashMap::new();
    for (index, run) in runs.iter().enumerate() {
        if let Some(parent_run_id) = run.parent_run_id {
            children_by_parent
                .entry(parent_run_id)
                .or_default()
                .push(index);
        }
    }

    let mut totals = HashMap::new();
    for index in 0..runs.len() {
        let (produced, completed) = accumulate(index, runs, &children_by_parent, &mut totals);
        runs[index].nr_produced_samples_including_children = produced;
        runs[index].nr_completed_samples_including_children = completed;
    }
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

pub(crate) async fn get_runtime_logs(
    pool: &PgPool,
    limit: i64,
    source: Option<&str>,
    run_id: Option<i32>,
    include_child_runs: bool,
    node_name: Option<&str>,
    node_uuid: Option<&str>,
    level: Option<&str>,
    query: Option<&str>,
    before_id: Option<i64>,
) -> Result<RuntimeLogPage, sqlx::Error> {
    let query_pattern = query.map(|value| format!("%{value}%"));
    let rows = sqlx::query_as::<_, WorkerLogRow>(
        r#"
        SELECT
            id,
            ts,
            source,
            run_id,
            node_uuid,
            node_name,
            level,
            target,
            message,
            fields
        FROM (
            SELECT
                id,
                ts,
                source,
                run_id,
                node_uuid,
                node_name,
                level,
                target,
                message,
                fields
            FROM runtime_logs
            WHERE ($1::text IS NULL OR source = $1)
              AND (
                  $2::int IS NULL
                  OR run_id = $2
                  OR (
                      $3::bool
                      AND run_id IN (
                          WITH RECURSIVE child_runs(id) AS (
                              SELECT id FROM runs WHERE parent_run_id = $2
                              UNION ALL
                              SELECT runs.id
                              FROM runs
                              JOIN child_runs ON runs.parent_run_id = child_runs.id
                          )
                          SELECT id FROM child_runs
                      )
                  )
              )
              AND ($4::text IS NULL OR node_name = $4)
              AND ($5::text IS NULL OR node_uuid = $5)
              AND ($6::text IS NULL OR level = $6)
              AND ($7::text IS NULL OR message ILIKE $7 OR target ILIKE $7 OR fields::text ILIKE $7)
              AND ($8::bigint IS NULL OR id < $8)
            ORDER BY id DESC
            LIMIT $9
        ) recent
        ORDER BY id DESC
        "#,
    )
    .bind(source)
    .bind(run_id)
    .bind(include_child_runs)
    .bind(node_name)
    .bind(node_uuid)
    .bind(level)
    .bind(query_pattern)
    .bind(before_id)
    .bind(limit + 1)
    .fetch_all(pool)
    .await?;

    let has_more_older = rows.len() as i64 > limit;
    let items: Vec<RuntimeLogEntry> = rows
        .into_iter()
        .take(limit as usize)
        .map(Into::into)
        .collect();
    let next_before_id = if has_more_older {
        items.last().map(|entry| entry.id.clone())
    } else {
        None
    };

    Ok(RuntimeLogPage {
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
                    n.capabilities,
                    n.desired_run_id,
                    dr.name AS desired_run_name,
                    n.desired_role,
                    n.active_run_id AS current_run_id,
                    cr.name AS current_run_name,
                    n.active_role AS current_role,
                    n.last_seen,
                    e.metrics AS evaluator_metrics,
                    e.rss_bytes AS evaluator_rss_bytes,
                    p.metrics AS sampler_metrics,
                    p.runtime_metrics AS sampler_runtime_metrics,
                    p.engine_diagnostics AS sampler_engine_diagnostics,
                    p.rss_bytes AS sampler_rss_bytes
                FROM nodes n
                LEFT JOIN sampler_aggregator_performance_latest p
                    ON p.run_id = COALESCE($1::integer, n.active_run_id, n.desired_run_id)
                   AND p.worker_id = n.name
                LEFT JOIN evaluator_performance_latest e
                    ON e.run_id = COALESCE($1::integer, n.active_run_id, n.desired_run_id)
                   AND e.worker_id = n.name
                LEFT JOIN runs dr ON dr.id = n.desired_run_id
                LEFT JOIN runs cr ON cr.id = n.active_run_id
                WHERE n.lease_expires_at > now()
                  AND ($1::integer IS NULL OR n.desired_run_id = $1 OR n.active_run_id = $1)
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

pub(crate) async fn get_registered_worker(
    pool: &PgPool,
    node_name: &str,
) -> Result<Option<RegisteredWorkerEntry>, sqlx::Error> {
    sqlx::query_as::<_, RegisteredWorkerRow>(
        r#"
        SELECT
            n.name AS node_name,
            n.uuid AS node_uuid,
            n.capabilities,
            n.desired_run_id,
            dr.name AS desired_run_name,
            n.desired_role,
            n.active_run_id AS current_run_id,
            cr.name AS current_run_name,
            n.active_role AS current_role,
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
        WHERE n.name = $1 AND n.lease_expires_at > now()
        "#,
    )
    .bind(node_name)
    .fetch_optional(pool)
    .await
    .map(|row| row.map(Into::into))
}

pub(crate) async fn get_registered_worker_summaries(
    pool: &PgPool,
    run_id: Option<i32>,
) -> Result<Vec<RegisteredWorkerSummary>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RegisteredWorkerSummaryRow>(
        r#"
                SELECT
                    n.name AS node_name,
                    n.uuid AS node_uuid,
                    n.capabilities,
                    n.desired_run_id,
                    dr.name AS desired_run_name,
                    n.desired_role,
                    n.active_run_id AS current_run_id,
                    cr.name AS current_run_name,
                    n.active_role AS current_role,
                    n.last_seen
                FROM nodes n
                LEFT JOIN runs dr ON dr.id = n.desired_run_id
                LEFT JOIN runs cr ON cr.id = n.active_run_id
                WHERE n.lease_expires_at > now()
                  AND ($1::integer IS NULL OR n.desired_run_id = $1 OR n.active_run_id = $1)
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
