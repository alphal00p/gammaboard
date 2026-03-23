use serde_json::Value as JsonValue;
use sqlx::{Executor, PgPool, Postgres};
use std::collections::HashMap;

use crate::core::{
    BatchTransformConfig, MaterializerState, RunStageSnapshot, SamplerAggregatorConfig,
};
use crate::evaluation::ObservableState;
use crate::runners::sampler_aggregator::SamplerAggregatorRunnerSnapshot;
use crate::sampling::SamplerAggregatorSnapshot;

#[derive(sqlx::FromRow)]
struct RunStageSnapshotRow {
    id: i64,
    run_id: i32,
    task_id: Option<i64>,
    sequence_nr: Option<i32>,
    queue_empty: bool,
    sampler_snapshot: JsonValue,
    observable_state: Option<JsonValue>,
    sampler_aggregator: JsonValue,
    materializer: JsonValue,
    batch_transforms: JsonValue,
}

impl TryFrom<RunStageSnapshotRow> for RunStageSnapshot {
    type Error = sqlx::Error;

    fn try_from(value: RunStageSnapshotRow) -> Result<Self, Self::Error> {
        let decode = |field: &str, err: serde_json::Error| {
            sqlx::Error::Protocol(format!(
                "failed to decode {field} from run_stage_snapshots: {err}"
            ))
        };
        Ok(Self {
            id: Some(value.id),
            run_id: value.run_id,
            task_id: value.task_id,
            sequence_nr: value.sequence_nr,
            queue_empty: value.queue_empty,
            sampler_snapshot: serde_json::from_value::<SamplerAggregatorSnapshot>(
                value.sampler_snapshot,
            )
            .map_err(|err| decode("sampler_snapshot", err))?,
            observable_state: value
                .observable_state
                .map(|payload| {
                    ObservableState::from_json(&payload).map_err(|err| {
                        sqlx::Error::Protocol(format!(
                            "failed to decode observable_state from run_stage_snapshots: {err}"
                        ))
                    })
                })
                .transpose()?,
            sampler_aggregator: serde_json::from_value::<SamplerAggregatorConfig>(
                value.sampler_aggregator,
            )
            .map_err(|err| decode("sampler_aggregator", err))?,
            materializer: serde_json::from_value::<MaterializerState>(value.materializer)
                .map_err(|err| decode("materializer", err))?,
            batch_transforms: serde_json::from_value::<Vec<BatchTransformConfig>>(
                value.batch_transforms,
            )
            .map_err(|err| decode("batch_transforms", err))?,
        })
    }
}

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
) -> Result<Option<SamplerAggregatorRunnerSnapshot>, sqlx::Error> {
    let row: Option<JsonValue> = sqlx::query_scalar(
        r#"
        SELECT sampler_runner_snapshot
        FROM runs
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    row.map(|payload| {
        serde_json::from_value(payload).map_err(|err| {
            sqlx::Error::Protocol(format!("failed to decode sampler_runner_snapshot: {err}"))
        })
    })
    .transpose()
}

pub(crate) async fn get_latest_stage_snapshot_before_sequence(
    pool: &PgPool,
    run_id: i32,
    sequence_nr: i32,
) -> Result<Option<RunStageSnapshot>, sqlx::Error> {
    let row = sqlx::query_as::<_, RunStageSnapshotRow>(
        r#"
        SELECT
            id,
            run_id,
            task_id,
            sequence_nr,
            queue_empty,
            sampler_snapshot,
            observable_state,
            sampler_aggregator,
            materializer,
            batch_transforms
        FROM run_stage_snapshots
        WHERE run_id = $1
          AND queue_empty = TRUE
          AND (sequence_nr IS NULL OR sequence_nr < $2)
        ORDER BY sequence_nr DESC NULLS LAST, id DESC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .bind(sequence_nr)
    .fetch_optional(pool)
    .await?;
    row.map(TryInto::try_into).transpose()
}

pub(crate) async fn get_stage_snapshot(
    pool: &PgPool,
    snapshot_id: i64,
) -> Result<Option<RunStageSnapshot>, sqlx::Error> {
    let row = sqlx::query_as::<_, RunStageSnapshotRow>(
        r#"
        SELECT
            id,
            run_id,
            task_id,
            sequence_nr,
            queue_empty,
            sampler_snapshot,
            observable_state,
            sampler_aggregator,
            materializer,
            batch_transforms
        FROM run_stage_snapshots
        WHERE id = $1
        LIMIT 1
        "#,
    )
    .bind(snapshot_id)
    .fetch_optional(pool)
    .await?;
    row.map(TryInto::try_into).transpose()
}

pub(crate) async fn list_latest_stage_snapshot_ids_by_task(
    pool: &PgPool,
    run_id: i32,
) -> Result<HashMap<i64, i64>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT task_id, MAX(id) AS snapshot_id
        FROM run_stage_snapshots
        WHERE run_id = $1
          AND queue_empty = TRUE
          AND task_id IS NOT NULL
        GROUP BY task_id
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

pub(crate) async fn get_task_activation_stage_snapshot(
    pool: &PgPool,
    run_id: i32,
    task_id: i64,
) -> Result<Option<RunStageSnapshot>, sqlx::Error> {
    let row = sqlx::query_as::<_, RunStageSnapshotRow>(
        r#"
        SELECT
            id,
            run_id,
            task_id,
            sequence_nr,
            queue_empty,
            sampler_snapshot,
            observable_state,
            sampler_aggregator,
            materializer,
            batch_transforms
        FROM run_stage_snapshots
        WHERE run_id = $1
          AND task_id = $2
          AND queue_empty = TRUE
        ORDER BY created_at ASC, id ASC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .bind(task_id)
    .fetch_optional(pool)
    .await?;
    row.map(TryInto::try_into).transpose()
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
    snapshot: &SamplerAggregatorRunnerSnapshot,
) -> Result<(), sqlx::Error> {
    let payload = serde_json::to_value(snapshot).map_err(|err| {
        sqlx::Error::Protocol(format!("failed to encode sampler_runner_snapshot: {err}"))
    })?;
    sqlx::query(
        r#"
        UPDATE runs
        SET sampler_runner_snapshot = $1
        WHERE id = $2
        "#,
    )
    .bind(payload)
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

pub(crate) async fn insert_run_stage_snapshot<'a, E>(
    executor: E,
    snapshot: &RunStageSnapshot,
) -> Result<(), sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO run_stage_snapshots (
            run_id,
            task_id,
            sequence_nr,
            queue_empty,
            sampler_snapshot,
            observable_state,
            sampler_aggregator,
            materializer,
            batch_transforms
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(snapshot.run_id)
    .bind(snapshot.task_id)
    .bind(snapshot.sequence_nr)
    .bind(snapshot.queue_empty)
    .bind(
        serde_json::to_value(&snapshot.sampler_snapshot).map_err(|err| {
            sqlx::Error::Protocol(format!(
                "failed to encode sampler_snapshot for run_stage_snapshots: {err}"
            ))
        })?,
    )
    .bind(
        snapshot
            .observable_state
            .as_ref()
            .map(|observable| {
                observable.to_json().map_err(|err| {
                    sqlx::Error::Protocol(format!(
                        "failed to encode observable_state for run_stage_snapshots: {err}"
                    ))
                })
            })
            .transpose()?,
    )
    .bind(
        serde_json::to_value(&snapshot.sampler_aggregator).map_err(|err| {
            sqlx::Error::Protocol(format!(
                "failed to encode sampler_aggregator for run_stage_snapshots: {err}"
            ))
        })?,
    )
    .bind(serde_json::to_value(&snapshot.materializer).map_err(|err| {
        sqlx::Error::Protocol(format!(
            "failed to encode materializer for run_stage_snapshots: {err}"
        ))
    })?)
    .bind(
        serde_json::to_value(&snapshot.batch_transforms).map_err(|err| {
            sqlx::Error::Protocol(format!(
                "failed to encode batch_transforms for run_stage_snapshots: {err}"
            ))
        })?,
    )
    .execute(executor)
    .await?;
    Ok(())
}
