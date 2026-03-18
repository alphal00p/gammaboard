use serde_json::Value as JsonValue;
use sqlx::PgPool;

use crate::core::RunStageSnapshot;

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

pub(crate) async fn get_run_sample_progress(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<(i64, i64)>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            nr_produced_samples,
            nr_completed_samples
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn insert_persisted_observable_snapshot(
    pool: &PgPool,
    run_id: i32,
    task_id: i64,
    persisted_observable: &JsonValue,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO persisted_observable_snapshots (run_id, task_id, persisted_observable)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(run_id)
    .bind(task_id)
    .bind(persisted_observable)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn update_run_current_observable(
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

pub(crate) async fn update_run_sample_progress(
    pool: &PgPool,
    run_id: i32,
    nr_produced_samples: i64,
    nr_completed_samples: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE runs
        SET
            nr_produced_samples = $1,
            nr_completed_samples = $2
        WHERE id = $3
        "#,
    )
    .bind(nr_produced_samples)
    .bind(nr_completed_samples)
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn insert_run_stage_snapshot(
    pool: &PgPool,
    snapshot: &RunStageSnapshot,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO run_stage_snapshots (
            run_id,
            task_id,
            sequence_nr,
            queue_empty,
            sampler_runner_snapshot,
            observable_state,
            persisted_observable,
            sampler_aggregator,
            parametrization
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(snapshot.run_id)
    .bind(snapshot.task_id)
    .bind(snapshot.sequence_nr)
    .bind(snapshot.queue_empty)
    .bind(&snapshot.sampler_runner_snapshot)
    .bind(&snapshot.observable_state)
    .bind(&snapshot.persisted_observable)
    .bind(&snapshot.sampler_aggregator)
    .bind(&snapshot.parametrization)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn get_parametrization_state(
    pool: &PgPool,
    run_id: i32,
    version: i64,
) -> Result<Option<JsonValue>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT state
        FROM parametrization_states
        WHERE run_id = $1 AND version = $2
        "#,
    )
    .bind(run_id)
    .bind(version)
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten())
}

pub(crate) async fn get_latest_parametrization_state_version(
    pool: &PgPool,
    run_id: i32,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT version
        FROM parametrization_states
        WHERE run_id = $1
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .map(|row| row.flatten())
}

pub(crate) async fn upsert_parametrization_state(
    pool: &PgPool,
    run_id: i32,
    version: i64,
    state: &JsonValue,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO parametrization_states (run_id, version, state)
        VALUES ($1, $2, $3)
        ON CONFLICT (run_id, version)
        DO UPDATE SET state = EXCLUDED.state
        "#,
    )
    .bind(run_id)
    .bind(version)
    .bind(state)
    .execute(pool)
    .await?;
    Ok(())
}
