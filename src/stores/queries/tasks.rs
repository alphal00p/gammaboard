use crate::core::{RunTask, RunTaskSpec, RunTaskState};
use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::PgPool;

type RunTaskRow = (
    i64,
    i32,
    i32,
    JsonValue,
    Option<i64>,
    String,
    i64,
    i64,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
);

fn encode_task(task: &RunTaskSpec) -> Result<JsonValue, sqlx::Error> {
    serde_json::to_value(task)
        .map_err(|err| sqlx::Error::Protocol(format!("failed to serialize run task: {err}")))
}

fn decode_task_row(row: RunTaskRow) -> Result<RunTask, sqlx::Error> {
    let (
        id,
        run_id,
        sequence_nr,
        task,
        spawned_from_snapshot_id,
        state,
        nr_produced_samples,
        nr_completed_samples,
        failure_reason,
        started_at,
        completed_at,
        failed_at,
        created_at,
    ) = row;
    let task: RunTaskSpec =
        serde_json::from_value(task).map_err(|err| sqlx::Error::Decode(Box::new(err)))?;
    let state = match state.as_str() {
        "pending" => RunTaskState::Pending,
        "active" => RunTaskState::Active,
        "completed" => RunTaskState::Completed,
        "failed" => RunTaskState::Failed,
        other => {
            return Err(sqlx::Error::Protocol(format!(
                "unknown run task state from database: {other}"
            )));
        }
    };
    Ok(RunTask {
        id,
        run_id,
        sequence_nr,
        task,
        spawned_from_snapshot_id,
        state,
        nr_produced_samples,
        nr_completed_samples,
        failure_reason,
        started_at,
        completed_at,
        failed_at,
        created_at,
    })
}

pub(crate) async fn append_run_tasks(
    pool: &PgPool,
    run_id: i32,
    tasks: &[RunTaskSpec],
) -> Result<Vec<RunTask>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let next_sequence = sqlx::query_scalar::<_, i32>(
        r#"
        SELECT COALESCE(MAX(sequence_nr), 0) + 1
        FROM run_tasks
        WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_one(&mut *tx)
    .await?;

    let mut inserted = Vec::with_capacity(tasks.len());
    for (offset, task) in tasks.iter().enumerate() {
        let row = sqlx::query_as::<_, RunTaskRow>(
            r#"
            INSERT INTO run_tasks (
                run_id,
                sequence_nr,
                task,
                state
            )
            VALUES ($1, $2, $3, 'pending')
            RETURNING
                id,
                run_id,
                sequence_nr,
                task,
                spawned_from_snapshot_id,
                state,
                nr_produced_samples,
                nr_completed_samples,
                failure_reason,
                started_at,
                completed_at,
                failed_at,
                created_at
            "#,
        )
        .bind(run_id)
        .bind(next_sequence + offset as i32)
        .bind(encode_task(task)?)
        .fetch_one(&mut *tx)
        .await?;
        inserted.push(decode_task_row(row)?);
    }
    tx.commit().await?;
    Ok(inserted)
}

pub(crate) async fn list_run_tasks(
    pool: &PgPool,
    run_id: i32,
) -> Result<Vec<RunTask>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RunTaskRow>(
        r#"
        SELECT
            id,
            run_id,
            sequence_nr,
            task,
            spawned_from_snapshot_id,
            state,
            nr_produced_samples,
            nr_completed_samples,
            failure_reason,
            started_at,
            completed_at,
            failed_at,
            created_at
        FROM run_tasks
        WHERE run_id = $1
        ORDER BY sequence_nr ASC, id ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(decode_task_row).collect()
}

pub(crate) async fn remove_pending_run_task(
    pool: &PgPool,
    run_id: i32,
    task_id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM run_tasks
        WHERE id = $1
          AND run_id = $2
          AND state = 'pending'
        "#,
    )
    .bind(task_id)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub(crate) async fn load_active_run_task(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<RunTask>, sqlx::Error> {
    let row = sqlx::query_as::<_, RunTaskRow>(
        r#"
        SELECT
            id,
            run_id,
            sequence_nr,
            task,
            spawned_from_snapshot_id,
            state,
            nr_produced_samples,
            nr_completed_samples,
            failure_reason,
            started_at,
            completed_at,
            failed_at,
            created_at
        FROM run_tasks
        WHERE run_id = $1
          AND state = 'active'
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    row.map(decode_task_row).transpose()
}

pub(crate) async fn activate_next_run_task(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<RunTask>, sqlx::Error> {
    let row = sqlx::query_as::<_, RunTaskRow>(
        r#"
        WITH next_task AS (
            SELECT id
            FROM run_tasks
            WHERE run_id = $1
              AND state = 'pending'
            ORDER BY sequence_nr ASC, id ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        )
        UPDATE run_tasks
        SET
            state = 'active',
            started_at = COALESCE(started_at, now())
        WHERE id IN (SELECT id FROM next_task)
        RETURNING
            id,
            run_id,
            sequence_nr,
            task,
            spawned_from_snapshot_id,
            state,
            nr_produced_samples,
            nr_completed_samples,
            failure_reason,
            started_at,
            completed_at,
            failed_at,
            created_at
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    row.map(decode_task_row).transpose()
}

pub(crate) async fn update_run_task_progress(
    pool: &PgPool,
    task_id: i64,
    nr_produced_samples: i64,
    nr_completed_samples: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE run_tasks
        SET
            nr_produced_samples = $2,
            nr_completed_samples = $3
        WHERE id = $1
          AND state = 'active'
        "#,
    )
    .bind(task_id)
    .bind(nr_produced_samples)
    .bind(nr_completed_samples)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn set_run_task_spawn_origin(
    pool: &PgPool,
    task_id: i64,
    spawned_from_snapshot_id: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE run_tasks
        SET
            spawned_from_snapshot_id = $2
        WHERE id = $1
        "#,
    )
    .bind(task_id)
    .bind(spawned_from_snapshot_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn complete_run_task(pool: &PgPool, task_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE run_tasks
        SET
            state = 'completed',
            completed_at = now(),
            failure_reason = NULL,
            failed_at = NULL
        WHERE id = $1
          AND state = 'active'
        "#,
    )
    .bind(task_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn fail_run_task(
    pool: &PgPool,
    task_id: i64,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE run_tasks
        SET
            state = 'failed',
            failure_reason = $2,
            failed_at = now()
        WHERE id = $1
          AND state = 'active'
        "#,
    )
    .bind(task_id)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}
