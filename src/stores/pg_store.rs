//! Postgres-backed implementations of store contracts.

use super::sql_queries as queries;
use crate::contracts::{
    AggregationStore, AssignmentLeaseStore, BatchClaim, CompletedBatch, Worker,
    WorkerRegistryStore, WorkerRole, ControlPlaneStore, DesiredAssignment, EngineState,
    EngineStateStore, WorkerStatus, RunReadStore, RunSpec, RunSpecStore, StoreError,
    WorkQueueStore,
};
use crate::{Batch, BatchResults};
use chrono::{DateTime, Utc};
use serde_json::{Value as JsonValue, json};
use sqlx::{PgPool, Row};
use std::time::Duration;

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

fn store_err(message: impl Into<String>) -> StoreError {
    StoreError::new(message)
}

fn map_sqlx(err: sqlx::Error) -> StoreError {
    store_err(err.to_string())
}

fn role_to_str(role: WorkerRole) -> &'static str {
    match role {
        WorkerRole::Evaluator => "evaluator",
        WorkerRole::SamplerAggregator => "sampler_aggregator",
    }
}

fn parse_role(value: &str) -> Result<WorkerRole, StoreError> {
    match value {
        "evaluator" => Ok(WorkerRole::Evaluator),
        "sampler_aggregator" => Ok(WorkerRole::SamplerAggregator),
        other => Err(store_err(format!("unknown worker role: {other}"))),
    }
}

fn status_to_str(status: WorkerStatus) -> &'static str {
    match status {
        WorkerStatus::Active => "active",
        WorkerStatus::Draining => "draining",
        WorkerStatus::Inactive => "inactive",
    }
}

fn parse_status(value: &str) -> Result<WorkerStatus, StoreError> {
    match value {
        "active" => Ok(WorkerStatus::Active),
        "draining" => Ok(WorkerStatus::Draining),
        "inactive" => Ok(WorkerStatus::Inactive),
        other => Err(store_err(format!("unknown worker status: {other}"))),
    }
}

fn control_plane_worker_id(node_id: &str, role: WorkerRole) -> String {
    format!("{node_id}-{}", role_to_str(role))
}

#[derive(Debug, Clone, Copy)]
struct DeltaAggregation {
    nr_samples: i64,
    nr_batches: i32,
    sum: f64,
    sum_x2: f64,
    sum_abs: f64,
    max: Option<f64>,
    min: Option<f64>,
    weighted_sum: f64,
    weighted_sum_x2: f64,
    sum_weights: f64,
}

impl Default for DeltaAggregation {
    fn default() -> Self {
        Self {
            nr_samples: 0,
            nr_batches: 0,
            sum: 0.0,
            sum_x2: 0.0,
            sum_abs: 0.0,
            max: None,
            min: None,
            weighted_sum: 0.0,
            weighted_sum_x2: 0.0,
            sum_weights: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SnapshotAggregate {
    nr_samples: i64,
    nr_batches: i32,
    sum: f64,
    sum_x2: f64,
    sum_abs: f64,
    max: Option<f64>,
    min: Option<f64>,
    weighted_sum: f64,
    weighted_sum_x2: f64,
    sum_weights: f64,
    mean: Option<f64>,
    variance: Option<f64>,
    std_dev: Option<f64>,
    error_estimate: Option<f64>,
}

fn aggregate_completed_batches(completed: &[CompletedBatch]) -> DeltaAggregation {
    let mut delta = DeltaAggregation::default();

    for batch in completed {
        if batch.results.values.len() != batch.batch.points.len() {
            continue;
        }

        delta.nr_batches += 1;
        for (point, value) in batch.batch.points.iter().zip(batch.results.values.iter()) {
            let v = *value;
            let w = point.weight;

            delta.nr_samples += 1;
            delta.sum += v;
            delta.sum_x2 += v * v;
            delta.sum_abs += v.abs();

            delta.weighted_sum += v * w;
            delta.weighted_sum_x2 += (v * w) * (v * w);
            delta.sum_weights += w;

            delta.max = Some(delta.max.map_or(v, |m| m.max(v)));
            delta.min = Some(delta.min.map_or(v, |m| m.min(v)));
        }
    }

    delta
}

fn combine_aggregation(
    previous: Option<SnapshotAggregate>,
    delta: DeltaAggregation,
) -> SnapshotAggregate {
    let (
        mut nr_samples,
        mut nr_batches,
        mut sum,
        mut sum_x2,
        mut sum_abs,
        mut max,
        mut min,
        mut weighted_sum,
        mut weighted_sum_x2,
        mut sum_weights,
    ) = if let Some(prev) = previous {
        (
            prev.nr_samples,
            prev.nr_batches,
            prev.sum,
            prev.sum_x2,
            prev.sum_abs,
            prev.max,
            prev.min,
            prev.weighted_sum,
            prev.weighted_sum_x2,
            prev.sum_weights,
        )
    } else {
        (0, 0, 0.0, 0.0, 0.0, None, None, 0.0, 0.0, 0.0)
    };

    nr_samples += delta.nr_samples;
    nr_batches += delta.nr_batches;
    sum += delta.sum;
    sum_x2 += delta.sum_x2;
    sum_abs += delta.sum_abs;
    weighted_sum += delta.weighted_sum;
    weighted_sum_x2 += delta.weighted_sum_x2;
    sum_weights += delta.sum_weights;

    max = match (max, delta.max) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
        (None, None) => None,
    };

    min = match (min, delta.min) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
        (None, None) => None,
    };

    let mean = if nr_samples > 0 {
        Some(sum / nr_samples as f64)
    } else {
        None
    };

    let variance = if nr_samples > 1 {
        let mu = mean.unwrap_or(0.0);
        Some(((sum_x2 / nr_samples as f64) - (mu * mu)).max(0.0))
    } else {
        None
    };
    let std_dev = variance.map(|v| v.sqrt());
    let error_estimate = if let Some(sd) = std_dev {
        if nr_samples > 0 {
            Some(sd / (nr_samples as f64).sqrt())
        } else {
            None
        }
    } else {
        None
    };

    SnapshotAggregate {
        nr_samples,
        nr_batches,
        sum,
        sum_x2,
        sum_abs,
        max,
        min,
        weighted_sum,
        weighted_sum_x2,
        sum_weights,
        mean,
        variance,
        std_dev,
        error_estimate,
    }
}

impl RunReadStore for PgStore {
    async fn health_check(&self) -> Result<(), StoreError> {
        queries::health_check(&self.pool).await.map_err(map_sqlx)
    }

    async fn get_all_runs(&self) -> Result<Vec<crate::RunProgress>, StoreError> {
        queries::get_all_runs(&self.pool).await.map_err(map_sqlx)
    }

    async fn get_run_progress(
        &self,
        run_id: i32,
    ) -> Result<Option<crate::RunProgress>, StoreError> {
        queries::get_run_progress(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn get_work_queue_stats(
        &self,
        run_id: i32,
    ) -> Result<Vec<crate::WorkQueueStats>, StoreError> {
        queries::get_work_queue_stats(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn get_latest_aggregated_result(
        &self,
        run_id: i32,
    ) -> Result<Option<crate::AggregatedResult>, StoreError> {
        queries::get_latest_aggregated_result(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn get_aggregated_results(
        &self,
        run_id: i32,
        limit: i64,
    ) -> Result<Vec<crate::AggregatedResult>, StoreError> {
        queries::get_aggregated_results(&self.pool, run_id, limit)
            .await
            .map_err(map_sqlx)
    }
}

impl RunSpecStore for PgStore {
    async fn load_run_spec(&self, run_id: i32) -> Result<Option<RunSpec>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT integration_params
            FROM runs
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let integration_params: JsonValue = row
            .get::<Option<JsonValue>, _>("integration_params")
            .unwrap_or_else(|| json!({}));

        let worker_implementation = integration_params
            .get("worker_implementation")
            .and_then(|v| v.as_str())
            .or_else(|| {
                integration_params
                    .pointer("/worker/implementation")
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("default_worker")
            .to_string();
        let worker_version = integration_params
            .get("worker_version")
            .and_then(|v| v.as_str())
            .or_else(|| {
                integration_params
                    .pointer("/worker/version")
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("v1")
            .to_string();
        let worker_params = integration_params
            .get("worker_params")
            .cloned()
            .or_else(|| integration_params.pointer("/worker/params").cloned())
            .unwrap_or_else(|| json!({}));
        let sampler_aggregator_implementation = integration_params
            .get("sampler_aggregator_implementation")
            .and_then(|v| v.as_str())
            .or_else(|| {
                integration_params
                    .pointer("/sampler_aggregator/implementation")
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("default_sampler_aggregator")
            .to_string();
        let sampler_aggregator_version = integration_params
            .get("sampler_aggregator_version")
            .and_then(|v| v.as_str())
            .or_else(|| {
                integration_params
                    .pointer("/sampler_aggregator/version")
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("v1")
            .to_string();
        let sampler_aggregator_params = integration_params
            .get("sampler_aggregator_params")
            .cloned()
            .or_else(|| {
                integration_params
                    .pointer("/sampler_aggregator/params")
                    .cloned()
            })
            .unwrap_or_else(|| json!({}));
        let worker_runner_params = integration_params
            .get("worker_runner_params")
            .cloned()
            .or_else(|| integration_params.pointer("/worker_runner/params").cloned())
            .unwrap_or_else(|| json!({}));
        let sampler_aggregator_runner_params = integration_params
            .get("sampler_aggregator_runner_params")
            .cloned()
            .or_else(|| {
                integration_params
                    .pointer("/sampler_aggregator_runner/params")
                    .cloned()
            })
            .unwrap_or_else(|| json!({}));

        Ok(Some(RunSpec {
            run_id,
            worker_implementation,
            worker_version,
            worker_params,
            sampler_aggregator_implementation,
            sampler_aggregator_version,
            sampler_aggregator_params,
            worker_runner_params,
            sampler_aggregator_runner_params,
        }))
    }
}

impl WorkerRegistryStore for PgStore {
    async fn register_worker(&self, worker: &Worker) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO workers (
                worker_id,
                node_id,
                role,
                implementation,
                version,
                node_specs,
                status,
                last_seen
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, now())
            ON CONFLICT (worker_id) DO UPDATE
            SET
                node_id = EXCLUDED.node_id,
                role = EXCLUDED.role,
                implementation = EXCLUDED.implementation,
                version = EXCLUDED.version,
                node_specs = EXCLUDED.node_specs,
                status = EXCLUDED.status,
                last_seen = now(),
                updated_at = now()
            "#,
        )
        .bind(&worker.worker_id)
        .bind(&worker.node_id)
        .bind(role_to_str(worker.role))
        .bind(&worker.implementation)
        .bind(&worker.version)
        .bind(&worker.node_specs)
        .bind(status_to_str(worker.status))
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(())
    }

    async fn heartbeat_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE workers
            SET last_seen = now(), updated_at = now()
            WHERE worker_id = $1
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn update_worker_status(
        &self,
        worker_id: &str,
        worker_status: WorkerStatus,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE workers
            SET status = $2, updated_at = now()
            WHERE worker_id = $1
            "#,
        )
        .bind(worker_id)
        .bind(status_to_str(worker_status))
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn get_worker(
        &self,
        worker_id: &str,
    ) -> Result<Option<Worker>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT
                worker_id,
                node_id,
                role,
                implementation,
                version,
                node_specs,
                status,
                last_seen
            FROM workers
            WHERE worker_id = $1
            "#,
        )
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let role: String = row.get("role");
        let status: String = row.get("status");

        Ok(Some(Worker {
            worker_id: row.get("worker_id"),
            node_id: row.get("node_id"),
            role: parse_role(&role)?,
            implementation: row.get("implementation"),
            version: row.get("version"),
            node_specs: row.get("node_specs"),
            status: parse_status(&status)?,
            last_seen: row.get("last_seen"),
        }))
    }
}

impl AssignmentLeaseStore for PgStore {
    async fn acquire_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError> {
        let ttl_secs = ttl.as_secs_f64().max(1.0);

        let row = sqlx::query(
            r#"
            INSERT INTO run_sampler_aggregator_leases (
                run_id,
                worker_id,
                lease_expires_at
            ) VALUES (
                $1,
                $2,
                now() + make_interval(secs => $3)
            )
            ON CONFLICT (run_id) DO UPDATE
            SET
                worker_id = EXCLUDED.worker_id,
                lease_expires_at = EXCLUDED.lease_expires_at,
                updated_at = now()
            WHERE
                run_sampler_aggregator_leases.worker_id = EXCLUDED.worker_id
                OR run_sampler_aggregator_leases.lease_expires_at < now()
            RETURNING run_id
            "#,
        )
        .bind(run_id)
        .bind(worker_id)
        .bind(ttl_secs)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.is_some())
    }

    async fn renew_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError> {
        let ttl_secs = ttl.as_secs_f64().max(1.0);

        let row = sqlx::query(
            r#"
            UPDATE run_sampler_aggregator_leases
            SET
                lease_expires_at = now() + make_interval(secs => $3),
                updated_at = now()
            WHERE run_id = $1 AND worker_id = $2
            RETURNING run_id
            "#,
        )
        .bind(run_id)
        .bind(worker_id)
        .bind(ttl_secs)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.is_some())
    }

    async fn release_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            DELETE FROM run_sampler_aggregator_leases
            WHERE run_id = $1 AND worker_id = $2
            "#,
        )
        .bind(run_id)
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn assign_evaluator(&self, run_id: i32, worker_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO run_evaluator_assignments (
                run_id,
                worker_id,
                active,
                assigned_at
            ) VALUES (
                $1, $2, true, now()
            )
            ON CONFLICT (run_id, worker_id) DO UPDATE
            SET active = true, assigned_at = now()
            "#,
        )
        .bind(run_id)
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn unassign_evaluator(&self, run_id: i32, worker_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE run_evaluator_assignments
            SET active = false
            WHERE run_id = $1 AND worker_id = $2
            "#,
        )
        .bind(run_id)
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn list_assigned_evaluators(&self, run_id: i32) -> Result<Vec<String>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT worker_id
            FROM run_evaluator_assignments
            WHERE run_id = $1 AND active = true
            ORDER BY assigned_at ASC
            "#,
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(rows
            .into_iter()
            .map(|row| row.get::<String, _>("worker_id"))
            .collect())
    }
}

impl ControlPlaneStore for PgStore {
    async fn upsert_desired_assignment(
        &self,
        node_id: &str,
        role: WorkerRole,
        run_id: i32,
    ) -> Result<(), StoreError> {
        let worker_id = control_plane_worker_id(node_id, role);
        sqlx::query(
            r#"
            INSERT INTO workers (
                worker_id,
                node_id,
                role,
                implementation,
                version,
                node_specs,
                status,
                desired_run_id,
                desired_updated_at,
                updated_at
            ) VALUES (
                $1, $2, $3, 'control_plane', 'v1', '{}'::jsonb, 'inactive', $4, now(), now()
            )
            ON CONFLICT (worker_id) DO UPDATE
            SET
                node_id = EXCLUDED.node_id,
                role = EXCLUDED.role,
                desired_run_id = EXCLUDED.desired_run_id,
                desired_updated_at = now(),
                updated_at = now()
            "#,
        )
        .bind(&worker_id)
        .bind(node_id)
        .bind(role_to_str(role))
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn clear_desired_assignment(
        &self,
        node_id: &str,
        role: WorkerRole,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE workers
            SET
                desired_run_id = NULL,
                desired_updated_at = now(),
                updated_at = now()
            WHERE node_id = $1 AND role = $2
            "#,
        )
        .bind(node_id)
        .bind(role_to_str(role))
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn get_desired_assignment(
        &self,
        node_id: &str,
        role: WorkerRole,
    ) -> Result<Option<DesiredAssignment>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT desired_run_id AS run_id
            FROM workers
            WHERE node_id = $1 AND role = $2
              AND desired_run_id IS NOT NULL
            "#,
        )
        .bind(node_id)
        .bind(role_to_str(role))
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.map(|row| DesiredAssignment {
            node_id: node_id.to_string(),
            role,
            run_id: row.get("run_id"),
        }))
    }

    async fn list_desired_assignments(
        &self,
        node_id: Option<&str>,
    ) -> Result<Vec<DesiredAssignment>, StoreError> {
        let rows = if let Some(node_id) = node_id {
            sqlx::query(
                r#"
                SELECT node_id, role, desired_run_id AS run_id
                FROM workers
                WHERE node_id = $1
                  AND desired_run_id IS NOT NULL
                ORDER BY role ASC
                "#,
            )
            .bind(node_id)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
        } else {
            sqlx::query(
                r#"
                SELECT node_id, role, desired_run_id AS run_id
                FROM workers
                WHERE desired_run_id IS NOT NULL
                ORDER BY node_id ASC, role ASC
                "#,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
        };

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let role: String = row.get("role");
            out.push(DesiredAssignment {
                node_id: row.get("node_id"),
                role: parse_role(&role)?,
                run_id: row.get("run_id"),
            });
        }
        Ok(out)
    }

    async fn create_run(
        &self,
        status: &str,
        integration_params: &JsonValue,
    ) -> Result<i32, StoreError> {
        sqlx::query_scalar(
            r#"
            INSERT INTO runs (status, integration_params)
            VALUES ($1, $2)
            RETURNING id
            "#,
        )
        .bind(status)
        .bind(integration_params)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)
    }

    async fn set_run_status(&self, run_id: i32, status: &str) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE runs
            SET status = $2
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .bind(status)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if result.rows_affected() == 0 {
            return Err(store_err(format!("run {run_id} not found")));
        }
        Ok(())
    }

    async fn remove_run(&self, run_id: i32) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            DELETE FROM runs
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        if result.rows_affected() == 0 {
            return Err(store_err(format!("run {run_id} not found")));
        }
        Ok(())
    }
}

impl WorkQueueStore for PgStore {
    async fn insert_batch(&self, run_id: i32, batch: &Batch) -> Result<(), StoreError> {
        queries::insert_batch(&self.pool, run_id, batch)
            .await
            .map_err(map_sqlx)
    }

    async fn get_pending_batch_count(&self, run_id: i32) -> Result<i64, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS cnt
            FROM batches
            WHERE run_id = $1
              AND status = 'pending'
            "#,
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(row.get::<i64, _>("cnt"))
    }

    async fn claim_batch(
        &self,
        run_id: i32,
        worker_id: &str,
    ) -> Result<Option<BatchClaim>, StoreError> {
        let claimed = queries::claim_batch(&self.pool, run_id, worker_id)
            .await
            .map_err(map_sqlx)?;

        Ok(claimed.map(|(batch_id, batch)| BatchClaim { batch_id, batch }))
    }

    async fn submit_batch_results(
        &self,
        batch_id: i64,
        results: &BatchResults,
        eval_time_ms: f64,
    ) -> Result<(), StoreError> {
        queries::submit_batch_results(&self.pool, batch_id, results, eval_time_ms)
            .await
            .map_err(map_sqlx)
    }

    async fn fail_batch(&self, batch_id: i64, last_error: &str) -> Result<(), StoreError> {
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
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn fetch_completed_batches_since(
        &self,
        run_id: i32,
        last_batch_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<CompletedBatch>, StoreError> {
        let rows = if let Some(last_id) = last_batch_id {
            sqlx::query(
                r#"
                SELECT id, points, results, completed_at
                FROM batches
                WHERE run_id = $1
                  AND status = 'completed'
                  AND results IS NOT NULL
                  AND id > $2
                ORDER BY id ASC
                LIMIT $3
                "#,
            )
            .bind(run_id)
            .bind(last_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
        } else {
            sqlx::query(
                r#"
                SELECT id, points, results, completed_at
                FROM batches
                WHERE run_id = $1
                  AND status = 'completed'
                  AND results IS NOT NULL
                ORDER BY id ASC
                LIMIT $2
                "#,
            )
            .bind(run_id)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?
        };

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let batch_id: i64 = row.get("id");
            let points_json: JsonValue = row.get("points");
            let results_json: JsonValue = row.get("results");
            let completed_at: Option<DateTime<Utc>> = row.get("completed_at");

            let batch = Batch::from_json(&points_json).map_err(|err| {
                store_err(format!(
                    "failed to deserialize batch points for batch_id={batch_id}: {err}"
                ))
            })?;
            let results = BatchResults::from_json(&results_json).map_err(|err| {
                store_err(format!(
                    "failed to deserialize batch results for batch_id={batch_id}: {err}"
                ))
            })?;

            out.push(CompletedBatch {
                batch_id,
                batch,
                results,
                completed_at,
            });
        }

        Ok(out)
    }
}

impl EngineStateStore for PgStore {
    async fn load_engine_state(&self, run_id: i32) -> Result<Option<EngineState>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT state
            FROM sampler_states
            WHERE run_id = $1
            ORDER BY version DESC
            LIMIT 1
            "#,
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let state_json: JsonValue = row.get("state");
        match serde_json::from_value::<EngineState>(state_json.clone()) {
            Ok(state) => Ok(Some(state)),
            Err(_) => Ok(Some(EngineState {
                last_processed_batch_id: None,
                state: state_json,
            })),
        }
    }

    async fn save_engine_state(&self, run_id: i32, state: &EngineState) -> Result<(), StoreError> {
        let payload = serde_json::to_value(state).map_err(|err| {
            store_err(format!(
                "failed to serialize engine state for run_id={run_id}: {err}"
            ))
        })?;

        sqlx::query(
            r#"
            INSERT INTO sampler_states (
                run_id,
                version,
                state,
                nr_samples_trained,
                training_error
            )
            VALUES (
                $1,
                COALESCE((SELECT MAX(version) + 1 FROM sampler_states WHERE run_id = $1), 1),
                $2,
                NULL,
                NULL
            )
            "#,
        )
        .bind(run_id)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        Ok(())
    }
}

impl AggregationStore for PgStore {
    async fn aggregate_and_persist(
        &self,
        run_id: i32,
        completed: &[CompletedBatch],
    ) -> Result<(), StoreError> {
        if completed.is_empty() {
            return Ok(());
        }

        let delta = aggregate_completed_batches(completed);
        if delta.nr_samples == 0 {
            return Ok(());
        }

        let previous = queries::get_latest_aggregation_snapshot(&self.pool, run_id)
            .await
            .map_err(map_sqlx)?
            .map(
                |(
                    nr_samples,
                    nr_batches,
                    sum,
                    sum_x2,
                    sum_abs,
                    max,
                    min,
                    weighted_sum,
                    weighted_sum_x2,
                    sum_weights,
                    mean,
                    variance,
                    std_dev,
                    error_estimate,
                    _created_at,
                )| SnapshotAggregate {
                    nr_samples,
                    nr_batches,
                    sum,
                    sum_x2,
                    sum_abs,
                    max,
                    min,
                    weighted_sum,
                    weighted_sum_x2,
                    sum_weights,
                    mean,
                    variance,
                    std_dev,
                    error_estimate,
                },
            );

        let combined = combine_aggregation(previous, delta);

        queries::insert_aggregated_results_snapshot(
            &self.pool,
            run_id,
            combined.nr_samples,
            combined.nr_batches,
            combined.sum,
            combined.sum_x2,
            combined.sum_abs,
            combined.max,
            combined.min,
            combined.weighted_sum,
            combined.weighted_sum_x2,
            combined.sum_weights,
            combined.mean,
            combined.variance,
            combined.std_dev,
            combined.error_estimate,
        )
        .await
        .map_err(map_sqlx)?;

        let final_result = if combined.sum_weights > 0.0 {
            Some(combined.weighted_sum / combined.sum_weights)
        } else {
            combined.mean
        };

        queries::update_run_summary_from_snapshot(
            &self.pool,
            run_id,
            delta.nr_batches,
            final_result,
            combined.error_estimate,
        )
        .await
        .map_err(map_sqlx)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WeightedPoint;
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn completed(values: &[f64], weights: &[f64]) -> CompletedBatch {
        let points = weights
            .iter()
            .map(|w| WeightedPoint::new(json!(1.0), *w))
            .collect::<Vec<_>>();
        CompletedBatch {
            batch_id: 1,
            batch: Batch::new(points),
            results: BatchResults::new(values.to_vec()),
            completed_at: None,
        }
    }

    fn unique_id(prefix: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        format!("{prefix}-{nanos}")
    }

    async fn test_store() -> Option<PgStore> {
        let db_url = std::env::var("DATABASE_URL").ok()?;
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&db_url)
            .await
            .ok()?;
        Some(PgStore::new(pool))
    }

    #[test]
    fn aggregate_completed_batches_ignores_mismatched_results() {
        let good = completed(&[1.0, 2.0], &[1.0, 2.0]);
        let bad = CompletedBatch {
            batch_id: 2,
            batch: Batch::new(vec![WeightedPoint::new(json!(1.0), 1.0)]),
            results: BatchResults::new(vec![1.0, 2.0]),
            completed_at: None,
        };

        let delta = aggregate_completed_batches(&[good, bad]);
        assert_eq!(delta.nr_batches, 1);
        assert_eq!(delta.nr_samples, 2);
        assert!((delta.sum - 3.0).abs() < 1e-12);
        assert!((delta.weighted_sum - 5.0).abs() < 1e-12);
    }

    #[test]
    fn combine_aggregation_accumulates_previous_and_delta() {
        let previous = SnapshotAggregate {
            nr_samples: 2,
            nr_batches: 1,
            sum: 3.0,
            sum_x2: 5.0,
            sum_abs: 3.0,
            max: Some(2.0),
            min: Some(1.0),
            weighted_sum: 4.0,
            weighted_sum_x2: 8.0,
            sum_weights: 3.0,
            mean: Some(1.5),
            variance: Some(0.25),
            std_dev: Some(0.5),
            error_estimate: Some(0.25),
        };
        let delta = DeltaAggregation {
            nr_samples: 2,
            nr_batches: 1,
            sum: 7.0,
            sum_x2: 25.0,
            sum_abs: 7.0,
            max: Some(4.0),
            min: Some(3.0),
            weighted_sum: 10.0,
            weighted_sum_x2: 52.0,
            sum_weights: 2.0,
        };

        let combined = combine_aggregation(Some(previous), delta);
        assert_eq!(combined.nr_samples, 4);
        assert_eq!(combined.nr_batches, 2);
        assert_eq!(combined.max, Some(4.0));
        assert_eq!(combined.min, Some(1.0));
        assert!((combined.mean.unwrap_or_default() - 2.5).abs() < 1e-12);
    }

    #[tokio::test]
    #[ignore = "requires postgres with project migrations applied"]
    async fn claim_batch_requires_active_assignment() {
        let Some(store) = test_store().await else {
            return;
        };
        let worker_id = unique_id("test-worker");

        let run_id: i32 =
            sqlx::query_scalar("INSERT INTO runs (status) VALUES ('running') RETURNING id")
                .fetch_one(store.pool())
                .await
                .expect("insert run");

        sqlx::query(
            r#"
            INSERT INTO workers (
                worker_id, node_id, role, implementation, version, node_specs, status
            ) VALUES (
                $1, NULL, 'evaluator', 'test_impl', 'v1', '{}'::jsonb, 'active'
            )
            "#,
        )
        .bind(&worker_id)
        .execute(store.pool())
        .await
        .expect("insert worker");

        store
            .assign_evaluator(run_id, &worker_id)
            .await
            .expect("assign evaluator");

        let batch = Batch::new(vec![WeightedPoint::new(json!(1.0), 1.0)]);
        store
            .insert_batch(run_id, &batch)
            .await
            .expect("insert batch");

        let claimed = store
            .claim_batch(run_id, &worker_id)
            .await
            .expect("claim batch");
        assert!(
            claimed.is_some(),
            "assigned evaluator should be able to claim"
        );

        sqlx::query("DELETE FROM runs WHERE id = $1")
            .bind(run_id)
            .execute(store.pool())
            .await
            .expect("cleanup run");
        sqlx::query("DELETE FROM workers WHERE worker_id = $1")
            .bind(&worker_id)
            .execute(store.pool())
            .await
            .expect("cleanup worker");
    }

    #[tokio::test]
    #[ignore = "requires postgres with project migrations applied"]
    async fn claim_batch_rejects_unassigned_or_inactive_assignment() {
        let Some(store) = test_store().await else {
            return;
        };
        let worker_id = unique_id("test-worker");

        let run_id: i32 =
            sqlx::query_scalar("INSERT INTO runs (status) VALUES ('running') RETURNING id")
                .fetch_one(store.pool())
                .await
                .expect("insert run");

        sqlx::query(
            r#"
            INSERT INTO workers (
                worker_id, node_id, role, implementation, version, node_specs, status
            ) VALUES (
                $1, NULL, 'evaluator', 'test_impl', 'v1', '{}'::jsonb, 'active'
            )
            "#,
        )
        .bind(&worker_id)
        .execute(store.pool())
        .await
        .expect("insert worker");

        let batch = Batch::new(vec![WeightedPoint::new(json!(2.0), 1.0)]);
        store
            .insert_batch(run_id, &batch)
            .await
            .expect("insert batch");

        let unassigned_claim = store
            .claim_batch(run_id, &worker_id)
            .await
            .expect("claim batch while unassigned");
        assert!(
            unassigned_claim.is_none(),
            "unassigned evaluator should not be able to claim"
        );

        store
            .assign_evaluator(run_id, &worker_id)
            .await
            .expect("assign evaluator");
        store
            .unassign_evaluator(run_id, &worker_id)
            .await
            .expect("unassign evaluator");

        let inactive_claim = store
            .claim_batch(run_id, &worker_id)
            .await
            .expect("claim batch while inactive");
        assert!(
            inactive_claim.is_none(),
            "inactive assignment should not be able to claim"
        );

        sqlx::query("DELETE FROM runs WHERE id = $1")
            .bind(run_id)
            .execute(store.pool())
            .await
            .expect("cleanup run");
        sqlx::query("DELETE FROM workers WHERE worker_id = $1")
            .bind(&worker_id)
            .execute(store.pool())
            .await
            .expect("cleanup worker");
    }
}
