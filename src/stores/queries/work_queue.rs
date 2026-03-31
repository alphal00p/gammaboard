use crate::core::{EvaluatorPerformanceSnapshot, SamplerAggregatorPerformanceSnapshot};
use crate::evaluation::BatchResult;
use crate::sampling::LatentBatch;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::{PgPool, Postgres, QueryBuilder};

pub(crate) struct CompletedBatchRaw {
    pub batch_id: i64,
    pub task_id: i64,
    pub requires_training_values: bool,
    pub latent_batch: JsonValue,
    pub values: Option<JsonValue>,
    pub batch_observable: JsonValue,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_eval_time_ms: Option<f64>,
}

fn encode_json<T: Serialize>(label: &str, value: &T) -> Result<JsonValue, sqlx::Error> {
    serde_json::to_value(value)
        .map_err(|err| sqlx::Error::Protocol(format!("failed to serialize {label}: {err}")))
}

pub(crate) async fn insert_batches(
    pool: &PgPool,
    run_id: i32,
    task_id: i64,
    requires_training_values: bool,
    batches: &[LatentBatch],
) -> Result<Vec<i64>, sqlx::Error> {
    if batches.is_empty() {
        return Ok(Vec::new());
    }

    let mut tx = pool.begin().await?;
    let mut builder = QueryBuilder::<Postgres>::new(
        r#"
        INSERT INTO batches (
            run_id,
            task_id,
            requires_training_values,
            batch_size,
            status
        )
        "#,
    );
    builder.push_values(batches.iter(), |mut row, batch| {
        row.push_bind(run_id)
            .push_bind(task_id)
            .push_bind(requires_training_values)
            .push_bind(batch.nr_samples as i32)
            .push_bind("pending");
    });
    builder.push(" RETURNING id");
    let batch_ids = builder
        .build_query_scalar::<i64>()
        .fetch_all(&mut *tx)
        .await?;

    let mut input_builder = QueryBuilder::<Postgres>::new(
        r#"
        INSERT INTO batch_inputs (
            batch_id,
            latent_batch
        )
        "#,
    );
    input_builder.push_values(
        batch_ids.iter().zip(batches.iter()),
        |mut row, (batch_id, batch)| {
            row.push_bind(*batch_id).push_bind(batch.into_json());
        },
    );
    input_builder.build().execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(batch_ids)
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

pub(crate) async fn get_open_batch_count(pool: &PgPool, run_id: i32) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM batches
        WHERE run_id = $1
          AND status IN ('pending', 'claimed', 'completed')
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
    node_uuid: &str,
) -> Result<Option<(i64, i64, bool, LatentBatch)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, i64, bool, JsonValue)>(
        r#"
        WITH next_batch AS (
            SELECT b.id
            FROM batches b
            WHERE b.run_id = $2
              AND b.status = 'pending'
              AND EXISTS (
                  SELECT 1
                  FROM nodes n
                  WHERE n.uuid = $1
                    AND n.active_run_id = $2
                    AND n.active_role = 'evaluator'
                    AND n.lease_expires_at > now()
              )
            ORDER BY b.created_at, b.id
            LIMIT 1
            FOR UPDATE SKIP LOCKED
        ),
        claimed AS (
            UPDATE batches b
            SET status = 'claimed',
                claimed_by_node_name = (
                    SELECT n.name
                    FROM nodes n
                    WHERE n.uuid = $1
                ),
                claimed_by_node_uuid = $1,
                claimed_at = now()
            FROM next_batch n
            WHERE b.id = n.id
            RETURNING b.id, b.task_id, b.requires_training_values
        )
        SELECT c.id, c.task_id, c.requires_training_values, i.latent_batch
        FROM claimed c
        JOIN batch_inputs i ON i.batch_id = c.id
        "#,
    )
    .bind(node_uuid)
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    if let Some((batch_id, task_id, requires_training_values, latent_json)) = row {
        let batch =
            LatentBatch::from_json(&latent_json).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        Ok(Some((batch_id, task_id, requires_training_values, batch)))
    } else {
        Ok(None)
    }
}

pub(crate) async fn release_claimed_batches_for_worker(
    pool: &PgPool,
    run_id: i32,
    node_uuid: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE batches
        SET status = 'pending',
            claimed_by_node_name = NULL,
            claimed_by_node_uuid = NULL,
            claimed_at = NULL
        WHERE run_id = $1
          AND status = 'claimed'
          AND claimed_by_node_uuid = $2
        "#,
    )
    .bind(run_id)
    .bind(node_uuid)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn submit_batch_results(
    pool: &PgPool,
    batch_id: i64,
    node_uuid: &str,
    result: &BatchResult,
    eval_time_ms: f64,
) -> Result<(), sqlx::Error> {
    result
        .validate_json_safe()
        .map_err(|err| sqlx::Error::Protocol(format!("invalid batch result payload: {err}")))?;
    let observable = encode_json("batch observable", &result.observable)?;
    let values = result.values_to_json();
    let mut tx = pool.begin().await?;
    let update_result = sqlx::query(
        r#"
        UPDATE batches
        SET status = 'completed',
            completed_at = now()
        WHERE id = $1
          AND claimed_by_node_uuid = $2
        "#,
    )
    .bind(batch_id)
    .bind(node_uuid)
    .execute(&mut *tx)
    .await?;
    if update_result.rows_affected() == 0 {
        return Err(sqlx::Error::Protocol(format!(
            "batch {batch_id} is no longer owned by node uuid '{node_uuid}'"
        )));
    }
    sqlx::query(
        r#"
        INSERT INTO batch_results (
            batch_id,
            "values",
            batch_observable,
            total_eval_time_ms,
            completed_at
        )
        VALUES ($1, $2, $3, $4, now())
        "#,
    )
    .bind(batch_id)
    .bind(values)
    .bind(observable)
    .bind(eval_time_ms)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn reclaim_abandoned_batches(
    pool: &PgPool,
    run_id: i32,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE batches b
        SET
            status = 'pending',
            claimed_by_node_name = NULL,
            claimed_by_node_uuid = NULL,
            claimed_at = NULL,
            retry_count = COALESCE(retry_count, 0) + 1,
            last_error = 'abandoned evaluator claim reclaimed'
        WHERE b.run_id = $1
          AND b.status = 'claimed'
          AND NOT EXISTS (
              SELECT 1
              FROM nodes n
              WHERE n.name = b.claimed_by_node_name
                AND n.uuid = b.claimed_by_node_uuid
                AND n.active_run_id = b.run_id
                AND n.active_role = 'evaluator'
                AND n.lease_expires_at > now()
          )
        "#,
    )
    .bind(run_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn insert_evaluator_performance_snapshot(
    pool: &PgPool,
    snapshot: &EvaluatorPerformanceSnapshot,
) -> Result<(), sqlx::Error> {
    let metrics = encode_json("evaluator performance metrics", &snapshot.metrics)?;
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
    .bind(&snapshot.node_name)
    .bind(&metrics)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn insert_sampler_aggregator_performance_snapshot(
    pool: &PgPool,
    snapshot: &SamplerAggregatorPerformanceSnapshot,
) -> Result<(), sqlx::Error> {
    let metrics = encode_json(
        "sampler performance metrics",
        &snapshot.runtime_metrics.to_performance_metrics(),
    )?;
    let runtime_metrics = encode_json("sampler runtime metrics", &snapshot.runtime_metrics)?;
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
    .bind(&snapshot.node_name)
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
    strict_ordering: bool,
) -> Result<Vec<CompletedBatchRaw>, sqlx::Error> {
    let rows = if strict_ordering {
        // Return only the contiguous completed prefix by id.
        // This keeps ingestion strictly ordered across batches and leaves out-of-order
        // completions buffered in the DB until gaps are resolved.
        sqlx::query_as::<
            _,
            (
                i64,
                i64,
                bool,
                JsonValue,
                Option<JsonValue>,
                JsonValue,
                Option<DateTime<Utc>>,
                Option<f64>,
            ),
        >(
            r#"
            WITH ordered AS (
                SELECT
                    b.id,
                    b.task_id,
                    b.requires_training_values,
                    b.status,
                    ROW_NUMBER() OVER (ORDER BY b.id ASC) AS rn
                FROM batches b
                WHERE b.run_id = $1
                ORDER BY b.id ASC
                LIMIT $2
            ),
            first_blocker AS (
                SELECT MIN(rn) AS rn
                FROM ordered
                WHERE status <> 'completed'
            )
            SELECT
                o.id,
                o.task_id,
                o.requires_training_values,
                i.latent_batch,
                r."values",
                r.batch_observable,
                r.completed_at,
                r.total_eval_time_ms
            FROM ordered o
            JOIN batch_inputs i ON i.batch_id = o.id
            JOIN batch_results r ON r.batch_id = o.id
            CROSS JOIN first_blocker b
            WHERE o.status = 'completed'
              AND (b.rn IS NULL OR o.rn < b.rn)
            ORDER BY o.id ASC
            "#,
        )
        .bind(run_id)
        .bind(limit as i64)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<
            _,
            (
                i64,
                i64,
                bool,
                JsonValue,
                Option<JsonValue>,
                JsonValue,
                Option<DateTime<Utc>>,
                Option<f64>,
            ),
        >(
            r#"
            SELECT
                b.id,
                b.task_id,
                b.requires_training_values,
                i.latent_batch,
                r."values",
                r.batch_observable,
                r.completed_at,
                r.total_eval_time_ms
            FROM batches b
            JOIN batch_inputs i ON i.batch_id = b.id
            JOIN batch_results r ON r.batch_id = b.id
            WHERE b.run_id = $1
              AND b.status = 'completed'
            ORDER BY b.id ASC
            LIMIT $2
            "#,
        )
        .bind(run_id)
        .bind(limit as i64)
        .fetch_all(pool)
        .await?
    };

    Ok(rows
        .into_iter()
        .map(
            |(
                batch_id,
                task_id,
                requires_training_values,
                latent_batch,
                values,
                batch_observable,
                completed_at,
                total_eval_time_ms,
            )| {
                CompletedBatchRaw {
                    batch_id,
                    task_id,
                    requires_training_values,
                    latent_batch,
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
        WHERE id = ANY($1)
          AND status = 'completed'
        "#,
    )
    .bind(batch_ids)
    .execute(pool)
    .await?;

    Ok(())
}
