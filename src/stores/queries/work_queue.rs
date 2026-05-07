use crate::core::{
    BatchFailOutcome, BatchQueueCounts, EvaluatorPerformanceSnapshot, InsertBatchesMetrics,
    InsertBatchesOutcome, SamplerAggregatorPerformanceSnapshot, StoreError,
};
use crate::evaluation::BatchResult;
use crate::sampling::LatentBatch;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::{PgPool, Postgres, QueryBuilder};
use std::time::Instant;

pub(crate) struct CompletedBatchRaw {
    pub batch_id: i64,
    pub task_id: i64,
    pub requires_training_values: bool,
    pub batch_size: i32,
    pub values: Option<Vec<u8>>,
    pub batch_observable: JsonValue,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_eval_time_ms: Option<f64>,
}

fn encode_json<T: Serialize>(label: &str, value: &T) -> Result<JsonValue, sqlx::Error> {
    serde_json::to_value(value)
        .map_err(|err| sqlx::Error::Protocol(format!("failed to serialize {label}: {err}")))
}

const PG_COPY_BINARY_HEADER: &[u8] = b"PGCOPY\n\xff\r\n\0";

fn encode_batch_inputs_copy_binary(serialized_inputs: &[(i64, Vec<u8>)]) -> Vec<u8> {
    let estimated_capacity = PG_COPY_BINARY_HEADER.len()
        + 4
        + 4
        + serialized_inputs
            .iter()
            .map(|(_, payload)| 2 + 4 + 8 + 4 + payload.len())
            .sum::<usize>()
        + 2;
    let mut out = Vec::with_capacity(estimated_capacity);
    out.extend_from_slice(PG_COPY_BINARY_HEADER);
    out.extend_from_slice(&0_i32.to_be_bytes());
    out.extend_from_slice(&0_i32.to_be_bytes());

    for (batch_id, payload) in serialized_inputs {
        out.extend_from_slice(&2_i16.to_be_bytes());

        out.extend_from_slice(&8_i32.to_be_bytes());
        out.extend_from_slice(&batch_id.to_be_bytes());

        out.extend_from_slice(&(payload.len() as i32).to_be_bytes());
        out.extend_from_slice(payload);
    }

    out.extend_from_slice(&(-1_i16).to_be_bytes());
    out
}

pub(crate) async fn insert_batches(
    pool: &PgPool,
    run_id: i32,
    task_id: i64,
    requires_training_values: bool,
    batch_ids: &[i64],
    batches: &[LatentBatch],
) -> Result<InsertBatchesOutcome, sqlx::Error> {
    if batches.is_empty() {
        return Ok(InsertBatchesOutcome::default());
    }
    if batch_ids.len() != batches.len() {
        return Err(sqlx::Error::Protocol(format!(
            "batch id count mismatch: ids={}, batches={}",
            batch_ids.len(),
            batches.len()
        )));
    }

    let started = Instant::now();
    let mut tx = pool.begin().await?;
    let mut builder = QueryBuilder::<Postgres>::new(
        r#"
        INSERT INTO batches (
            id,
            run_id,
            task_id,
            requires_training_values,
            batch_size,
            status
        )
        "#,
    );
    builder.push_values(
        batch_ids.iter().zip(batches.iter()),
        |mut row, (batch_id, batch)| {
            row.push_bind(*batch_id)
                .push_bind(run_id)
                .push_bind(task_id)
                .push_bind(requires_training_values)
                .push_bind(batch.nr_samples as i32)
                .push_bind("pending");
        },
    );
    let insert_batches_started = Instant::now();
    builder.build().execute(&mut *tx).await?;
    let insert_batches_exec_ms = insert_batches_started.elapsed().as_secs_f64() * 1000.0;

    let serialize_started = Instant::now();
    let serialized_inputs = batch_ids
        .iter()
        .zip(batches.iter())
        .map(|(batch_id, batch)| {
            batch
                .to_bytes()
                .map(|payload| (*batch_id, payload))
                .expect("latent batch serialization should never fail")
        })
        .collect::<Vec<_>>();
    let serialize_ms = serialize_started.elapsed().as_secs_f64() * 1000.0;
    let payload_bytes = serialized_inputs
        .iter()
        .map(|(_, payload)| payload.len())
        .sum::<usize>();

    let copy_payload = encode_batch_inputs_copy_binary(&serialized_inputs);
    let insert_inputs_started = Instant::now();
    let mut copy_in = tx
        .copy_in_raw(
            r#"
            COPY batch_inputs (
                batch_id,
                latent_batch
            ) FROM STDIN WITH (FORMAT binary)
            "#,
        )
        .await?;
    copy_in.send(copy_payload).await?;
    let copied_rows = copy_in.finish().await?;
    if copied_rows != serialized_inputs.len() as u64 {
        return Err(sqlx::Error::Protocol(format!(
            "COPY batch_inputs inserted {copied_rows} rows, expected {}",
            serialized_inputs.len()
        )));
    }
    let insert_inputs_exec_ms = insert_inputs_started.elapsed().as_secs_f64() * 1000.0;

    let commit_started = Instant::now();
    tx.commit().await?;
    let commit_ms = commit_started.elapsed().as_secs_f64() * 1000.0;

    Ok(InsertBatchesOutcome {
        batch_ids: batch_ids.to_vec(),
        metrics: InsertBatchesMetrics {
            serialize_ms,
            payload_bytes,
            insert_batches_exec_ms,
            insert_inputs_exec_ms,
            commit_ms,
            end_to_end_ms: started.elapsed().as_secs_f64() * 1000.0,
        },
    })
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

pub(crate) async fn get_batch_queue_counts(
    pool: &PgPool,
    run_id: i32,
    completed_after_batch_id: Option<i64>,
) -> Result<BatchQueueCounts, sqlx::Error> {
    let completed_after_batch_id = completed_after_batch_id.unwrap_or(0);
    let (pending, claimed, completed) = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE status = 'pending') AS pending,
            COUNT(*) FILTER (WHERE status = 'claimed') AS claimed,
            COUNT(*) FILTER (WHERE status = 'completed' AND id > $2) AS completed
        FROM batches
        WHERE run_id = $1
        "#,
    )
    .bind(run_id)
    .bind(completed_after_batch_id)
    .fetch_one(pool)
    .await?;
    Ok(BatchQueueCounts {
        pending,
        claimed,
        completed,
    })
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

pub(crate) async fn fetch_batches_by_status(
    pool: &PgPool,
    run_id: i32,
    status: &str,
    limit: i64,
) -> Result<
    Vec<(
        i64,
        i64,
        bool,
        i32,
        String,
        Option<String>,
        Option<String>,
        Vec<u8>,
    )>,
    sqlx::Error,
> {
    // returns tuple: (batch_id, task_id, requires_training_values, batch_size, status, claimed_by_node_name, claimed_by_node_uuid, latent_bytes)
    let rows = match status {
        "pending" => {
            sqlx::query_as::<_, (i64, i64, bool, i32, String, Option<String>, Option<String>, Vec<u8>)>(
                r#"
                SELECT b.id, b.task_id, b.requires_training_values, b.batch_size,
                       b.status::text, b.claimed_by_node_name, b.claimed_by_node_uuid, i.latent_batch
                FROM batches b
                JOIN batch_inputs i ON i.batch_id = b.id
                WHERE b.run_id = $1
                  AND b.status = 'pending'
                ORDER BY b.created_at, b.id
                LIMIT $2
                "#,
            )
            .bind(run_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        "claimed" => {
            sqlx::query_as::<_, (i64, i64, bool, i32, String, Option<String>, Option<String>, Vec<u8>)>(
                r#"
                SELECT b.id, b.task_id, b.requires_training_values, b.batch_size,
                       b.status::text, b.claimed_by_node_name, b.claimed_by_node_uuid, i.latent_batch
                FROM batches b
                JOIN batch_inputs i ON i.batch_id = b.id
                WHERE b.run_id = $1
                  AND b.status = 'claimed'
                ORDER BY b.created_at, b.id
                LIMIT $2
                "#,
            )
            .bind(run_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        _ => {
            sqlx::query_as::<_, (i64, i64, bool, i32, String, Option<String>, Option<String>, Vec<u8>)>(
                r#"
                SELECT b.id, b.task_id, b.requires_training_values, b.batch_size,
                       b.status::text, b.claimed_by_node_name, b.claimed_by_node_uuid, i.latent_batch
                FROM batches b
                JOIN batch_inputs i ON i.batch_id = b.id
                WHERE b.run_id = $1
                  AND b.status IN ('pending','claimed')
                ORDER BY b.created_at, b.id
                LIMIT $2
                "#,
            )
            .bind(run_id)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows)
}

pub(crate) async fn claim_batch(
    pool: &PgPool,
    run_id: i32,
    node_uuid: &str,
) -> Result<Option<(i64, i64, bool, LatentBatch)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, i64, bool, Vec<u8>)>(
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

    if let Some((batch_id, task_id, requires_training_values, latent_bytes)) = row {
        let batch =
            LatentBatch::from_bytes(&latent_bytes).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
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
) -> Result<(), StoreError> {
    result
        .validate_json_safe()
        .map_err(|err| StoreError::store(format!("invalid batch result payload: {err}")))?;
    let accumulator =
        encode_json("batch accumulator", &result.accumulator).map_err(StoreError::from)?;
    let values = result.values_to_bytes().map_err(|err| {
        StoreError::store(format!("failed to serialize batch training values: {err}"))
    })?;
    let mut tx = pool.begin().await.map_err(StoreError::from)?;
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
    .await
    .map_err(StoreError::from)?;
    if update_result.rows_affected() == 0 {
        return Err(StoreError::batch_ownership_lost(batch_id, node_uuid));
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
    .bind(accumulator)
    .bind(eval_time_ms)
    .execute(&mut *tx)
    .await
    .map_err(StoreError::from)?;
    tx.commit().await.map_err(StoreError::from)?;
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
    let row = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO evaluator_performance_history (
            run_id,
            worker_id,
            metrics,
            rss_bytes
        )
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(snapshot.run_id)
    .bind(&snapshot.node_name)
    .bind(&metrics)
    .bind(snapshot.rss_bytes)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO evaluator_performance_latest (
            run_id,
            worker_id,
            id,
            metrics,
            rss_bytes,
            created_at
        )
        VALUES ($1, $2, $3, $4, $5, now())
        ON CONFLICT (run_id, worker_id) DO UPDATE
        SET
            id = EXCLUDED.id,
            metrics = EXCLUDED.metrics,
            rss_bytes = EXCLUDED.rss_bytes,
            created_at = EXCLUDED.created_at
        "#,
    )
    .bind(snapshot.run_id)
    .bind(&snapshot.node_name)
    .bind(row)
    .bind(&metrics)
    .bind(snapshot.rss_bytes)
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
    let row = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO sampler_aggregator_performance_history (
            run_id,
            worker_id,
            metrics,
            runtime_metrics,
            engine_diagnostics,
            rss_bytes
        )
        VALUES (
            $1,
            $2,
            $3,
            $4,
            $5,
            $6
        )
        RETURNING id
        "#,
    )
    .bind(snapshot.run_id)
    .bind(&snapshot.node_name)
    .bind(&metrics)
    .bind(&runtime_metrics)
    .bind(&snapshot.engine_diagnostics)
    .bind(snapshot.rss_bytes)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO sampler_aggregator_performance_latest (
            run_id,
            worker_id,
            id,
            metrics,
            runtime_metrics,
            engine_diagnostics,
            rss_bytes,
            created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        ON CONFLICT (run_id, worker_id) DO UPDATE
        SET
            id = EXCLUDED.id,
            metrics = EXCLUDED.metrics,
            runtime_metrics = EXCLUDED.runtime_metrics,
            engine_diagnostics = EXCLUDED.engine_diagnostics,
            rss_bytes = EXCLUDED.rss_bytes,
            created_at = EXCLUDED.created_at
        "#,
    )
    .bind(snapshot.run_id)
    .bind(&snapshot.node_name)
    .bind(row)
    .bind(&metrics)
    .bind(&runtime_metrics)
    .bind(&snapshot.engine_diagnostics)
    .bind(snapshot.rss_bytes)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn fail_batch(
    pool: &PgPool,
    batch_id: i64,
    last_error: &str,
    max_batch_retries: i32,
) -> Result<BatchFailOutcome, sqlx::Error> {
    let row = sqlx::query_as::<_, (i64, i32, String)>(
        r#"
        UPDATE batches
        SET
            status = CASE
                WHEN COALESCE(retry_count, 0) + 1 >= $3 THEN 'failed'::batch_status
                ELSE 'pending'::batch_status
            END,
            last_error = $2,
            claimed_by_node_name = NULL,
            claimed_by_node_uuid = NULL,
            claimed_at = NULL,
            completed_at = NULL,
            retry_count = COALESCE(retry_count, 0) + 1
        WHERE id = $1
        RETURNING task_id, retry_count, status::text
        "#,
    )
    .bind(batch_id)
    .bind(last_error)
    .bind(max_batch_retries)
    .fetch_one(pool)
    .await?;
    let (task_id, retry_count, status) = row;
    if status == "failed" {
        Ok(BatchFailOutcome::PermanentlyFailed {
            task_id,
            retry_count,
        })
    } else {
        Ok(BatchFailOutcome::Requeued {
            task_id,
            retry_count,
        })
    }
}

pub(crate) async fn fetch_completed_batches(
    pool: &PgPool,
    run_id: i32,
    limit: usize,
    strict_ordering: bool,
    after_batch_id: Option<i64>,
) -> Result<Vec<CompletedBatchRaw>, sqlx::Error> {
    let after_batch_id = after_batch_id.unwrap_or(0);
    let rows = if strict_ordering {
        sqlx::query_as::<
            _,
            (
                i64,
                i64,
                bool,
                i32,
                Option<Vec<u8>>,
                Option<JsonValue>,
                Option<DateTime<Utc>>,
                Option<f64>,
            ),
        >(
            r#"
            WITH candidate_batches AS (
                SELECT
                    b.id,
                    b.task_id,
                    b.requires_training_values,
                    b.batch_size,
                    b.status
                FROM batches b
                WHERE b.run_id = $1
                  AND b.id > $2
                ORDER BY b.id ASC
                LIMIT $3
            ),
            first_incomplete AS (
                SELECT MIN(id) AS batch_id
                FROM candidate_batches
                WHERE status <> 'completed'
            ),
            completed_prefix AS (
                SELECT
                    c.id,
                    c.task_id,
                    c.requires_training_values,
                    c.batch_size
                FROM candidate_batches c
                CROSS JOIN first_incomplete f
                WHERE c.status = 'completed'
                  AND (f.batch_id IS NULL OR c.id < f.batch_id)
            )
            SELECT
                b.id,
                b.task_id,
                b.requires_training_values,
                b.batch_size,
                r."values",
                r.batch_observable,
                r.completed_at,
                r.total_eval_time_ms
            FROM completed_prefix b
            JOIN batch_results r ON r.batch_id = b.id
            ORDER BY b.id ASC
            "#,
        )
        .bind(run_id)
        .bind(after_batch_id)
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
                i32,
                Option<Vec<u8>>,
                Option<JsonValue>,
                Option<DateTime<Utc>>,
                Option<f64>,
            ),
        >(
            r#"
            SELECT
                b.id,
                b.task_id,
                b.requires_training_values,
                b.batch_size,
                r."values",
                r.batch_observable,
                r.completed_at,
                r.total_eval_time_ms
            FROM batches b
            JOIN batch_results r ON r.batch_id = b.id
            WHERE b.run_id = $1
              AND b.id > $2
              AND b.status = 'completed'
            ORDER BY b.id ASC
            LIMIT $3
            "#,
        )
        .bind(run_id)
        .bind(after_batch_id)
        .bind(limit as i64)
        .fetch_all(pool)
        .await?
    };

    let mut completed = Vec::new();
    for (
        batch_id,
        task_id,
        requires_training_values,
        batch_size,
        values,
        batch_observable,
        completed_at,
        total_eval_time_ms,
    ) in rows
    {
        let Some(batch_observable) = batch_observable else {
            return Err(sqlx::Error::Protocol(format!(
                "completed batch {batch_id} is missing persisted accumulator"
            )));
        };
        completed.push(CompletedBatchRaw {
            batch_id,
            task_id,
            requires_training_values,
            batch_size,
            values,
            batch_observable,
            completed_at,
            total_eval_time_ms,
        });
    }

    Ok(completed)
}

pub(crate) async fn cleanup_consumed_completed_batches(
    pool: &PgPool,
    run_id: i32,
    up_to_batch_id: i64,
    limit: usize,
) -> Result<u64, sqlx::Error> {
    if up_to_batch_id <= 0 || limit == 0 {
        return Ok(0);
    }

    let result = sqlx::query(
        r#"
        WITH cleanup_candidates AS (
            SELECT id
            FROM batches
            WHERE run_id = $1
              AND status = 'completed'
              AND id <= $2
            ORDER BY id ASC
            LIMIT $3
        )
        DELETE FROM batches b
        USING cleanup_candidates c
        WHERE b.id = c.id
        "#,
    )
    .bind(run_id)
    .bind(up_to_batch_id)
    .bind(limit as i64)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}
