//! Postgres-backed implementations of store contracts.

use super::queries;
use crate::core::{
    AggregationStore, AssignmentLeaseStore, BatchClaim, CompletedBatch, ControlPlaneStore,
    DesiredAssignment, EvaluatorPerformanceSnapshot, RunReadStore, RunSpecStore, RuntimeLogEvent,
    RuntimeLogStore, SamplerAggregatorPerformanceSnapshot, StoreError, WorkQueueStore, Worker,
    WorkerRegistryStore, WorkerRole, WorkerStatus,
};
use crate::core::{Batch, BatchResult, PointSpec};
use crate::engines::{IntegrationParams, RunSpec};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
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
    StoreError::store(message)
}

fn store_invalid_input(message: impl Into<String>) -> StoreError {
    StoreError::invalid_input(message)
}

fn map_sqlx(err: sqlx::Error) -> StoreError {
    StoreError::from(err)
}

const AGGREGATED_RANGE_MAX_POINTS_LIMIT: i64 = 10_000;

fn resolve_aggregated_range_index(raw: i64, latest_id: i64) -> Result<i64, StoreError> {
    if raw == 0 {
        // Accept 0 as a convenient alias for the first snapshot id.
        return Ok(1);
    }
    if raw > 0 {
        return Ok(raw);
    }
    Ok(latest_id + raw + 1)
}

fn ceil_div_i64(numerator: i64, denominator: i64) -> i64 {
    debug_assert!(denominator > 0);
    (numerator + denominator - 1) / denominator
}

fn next_power_of_two_i64(n: i64) -> i64 {
    if n <= 1 {
        return 1;
    }
    let mut p = 1_i64;
    while p < n {
        p = p.saturating_mul(2);
    }
    p
}

fn run_spec_from_integration_params(
    run_id: i32,
    point_spec: PointSpec,
    params: IntegrationParams,
) -> Result<RunSpec, StoreError> {
    Ok(RunSpec {
        run_id,
        point_spec,
        evaluator: params.evaluator,
        sampler_aggregator: params.sampler_aggregator,
        parametrization: params.parametrization,
        evaluator_runner_params: params.evaluator_runner_params,
        sampler_aggregator_runner_params: params.sampler_aggregator_runner_params,
    })
}

fn decode_run_spec(
    run_id: i32,
    integration_params: JsonValue,
    point_spec: JsonValue,
) -> Result<RunSpec, StoreError> {
    if !integration_params.is_object() {
        return Err(store_err(format!(
            "invalid integration_params payload for run_id={run_id}: expected object"
        )));
    }

    let params: IntegrationParams = serde_json::from_value(integration_params).map_err(|err| {
        store_err(format!(
            "invalid integration_params payload for run_id={run_id}: {err}"
        ))
    })?;

    let point_spec: PointSpec = serde_json::from_value(point_spec).map_err(|err| {
        store_err(format!(
            "invalid point_spec payload for run_id={run_id}: {err}"
        ))
    })?;

    run_spec_from_integration_params(run_id, point_spec, params)
}

fn parse_run_create_payload(integration_params: &JsonValue) -> Result<JsonValue, StoreError> {
    let root = integration_params.as_object().cloned().ok_or_else(|| {
        store_err("run create payload must be an object (integration_params json)")
    })?;

    Ok(JsonValue::Object(root))
}

#[async_trait::async_trait]
impl RunReadStore for PgStore {
    async fn health_check(&self) -> Result<(), StoreError> {
        queries::health_check(&self.pool).await?;
        Ok(())
    }

    async fn get_all_runs(&self) -> Result<Vec<crate::stores::RunProgress>, StoreError> {
        Ok(queries::get_all_runs(&self.pool).await?)
    }

    async fn get_run_progress(
        &self,
        run_id: i32,
    ) -> Result<Option<crate::stores::RunProgress>, StoreError> {
        Ok(queries::get_run_progress(&self.pool, run_id).await?)
    }

    async fn get_work_queue_stats(
        &self,
        run_id: i32,
    ) -> Result<Vec<crate::stores::WorkQueueStats>, StoreError> {
        Ok(queries::get_work_queue_stats(&self.pool, run_id).await?)
    }

    async fn get_latest_aggregated_result(
        &self,
        run_id: i32,
    ) -> Result<Option<crate::stores::AggregatedResult>, StoreError> {
        Ok(queries::get_latest_aggregated_result(&self.pool, run_id).await?)
    }

    async fn get_aggregated_results(
        &self,
        run_id: i32,
        limit: i64,
    ) -> Result<Vec<crate::stores::AggregatedResult>, StoreError> {
        Ok(queries::get_aggregated_results(&self.pool, run_id, limit).await?)
    }

    async fn get_aggregated_range(
        &self,
        run_id: i32,
        start: i64,
        stop: i64,
        max_points: i64,
        last_id: Option<i64>,
    ) -> Result<crate::stores::AggregatedRangeResponse, StoreError> {
        if max_points < 1 {
            return Err(store_invalid_input(
                "aggregated range max_points must be >= 1",
            ));
        }
        if max_points > AGGREGATED_RANGE_MAX_POINTS_LIMIT {
            return Err(store_invalid_input(format!(
                "aggregated range max_points must be <= {}",
                AGGREGATED_RANGE_MAX_POINTS_LIMIT
            )));
        }

        let Some((min_id, max_id)) = queries::get_aggregated_id_bounds(&self.pool, run_id).await?
        else {
            return Ok(crate::stores::AggregatedRangeResponse {
                snapshots: Vec::new(),
                latest: None,
                meta: crate::stores::AggregatedRangeMeta {
                    abs_start: None,
                    abs_stop: None,
                    step: 1,
                    latest_id: None,
                    max_points,
                },
                reset_required: false,
            });
        };

        let mut resolved_start = resolve_aggregated_range_index(start, max_id)?;
        let mut resolved_stop = resolve_aggregated_range_index(stop, max_id)?;
        if resolved_start > resolved_stop {
            return Err(store_invalid_input(format!(
                "aggregated range start > stop after resolution: start={} stop={}",
                resolved_start, resolved_stop
            )));
        }

        resolved_start = resolved_start.max(min_id).min(max_id);
        resolved_stop = resolved_stop.max(min_id).min(max_id);
        if resolved_start > resolved_stop {
            resolved_start = resolved_stop;
        }

        let span = (resolved_stop - resolved_start + 1).max(1);
        let min_step = ceil_div_i64(span, max_points);
        let step = next_power_of_two_i64(min_step);

        let grid_anchor = resolved_start;
        let reset_required = last_id.is_some_and(|seen| {
            seen < resolved_start || seen > resolved_stop || (seen - grid_anchor) % step != 0
        });
        let effective_last_id = if reset_required { None } else { last_id };

        let snapshots = queries::get_aggregated_range(
            &self.pool,
            run_id,
            resolved_start,
            resolved_stop,
            step,
            grid_anchor,
            effective_last_id,
            max_points,
        )
        .await?;

        let latest = queries::get_latest_aggregated_result(&self.pool, run_id).await?;

        Ok(crate::stores::AggregatedRangeResponse {
            snapshots,
            latest: latest.filter(|row| {
                effective_last_id.is_none_or(|seen| {
                    row.id
                        .parse::<i64>()
                        .ok()
                        .is_some_and(|latest_row_id| latest_row_id > seen)
                })
            }),
            meta: crate::stores::AggregatedRangeMeta {
                abs_start: Some(resolved_start),
                abs_stop: Some(resolved_stop),
                step,
                latest_id: Some(max_id.to_string()),
                max_points,
            },
            reset_required,
        })
    }

    async fn get_worker_logs(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
        level: Option<&str>,
        query: Option<&str>,
        before_id: Option<i64>,
    ) -> Result<crate::stores::WorkerLogPage, StoreError> {
        Ok(queries::get_worker_logs(
            &self.pool, run_id, limit, worker_id, level, query, before_id,
        )
        .await?)
    }

    async fn get_registered_workers(
        &self,
        run_id: Option<i32>,
    ) -> Result<Vec<crate::stores::RegisteredWorkerEntry>, StoreError> {
        Ok(queries::get_registered_workers(&self.pool, run_id).await?)
    }

    async fn get_evaluator_performance_history(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
    ) -> Result<Vec<crate::stores::EvaluatorPerformanceHistoryEntry>, StoreError> {
        Ok(
            queries::get_evaluator_performance_history(&self.pool, run_id, limit, worker_id)
                .await?,
        )
    }

    async fn get_sampler_performance_history(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
    ) -> Result<Vec<crate::stores::SamplerPerformanceHistoryEntry>, StoreError> {
        Ok(queries::get_sampler_performance_history(&self.pool, run_id, limit, worker_id).await?)
    }

    async fn get_worker_evaluator_performance_history(
        &self,
        worker_id: &str,
        limit: i64,
    ) -> Result<crate::stores::WorkerEvaluatorPerformanceHistoryResponse, StoreError> {
        let entries =
            queries::get_worker_evaluator_performance_history(&self.pool, worker_id, limit).await?;
        let run_id = entries.first().map(|entry| entry.run_id);
        Ok(crate::stores::WorkerEvaluatorPerformanceHistoryResponse { run_id, entries })
    }

    async fn get_worker_sampler_performance_history(
        &self,
        worker_id: &str,
        limit: i64,
    ) -> Result<crate::stores::WorkerSamplerPerformanceHistoryResponse, StoreError> {
        let entries =
            queries::get_worker_sampler_performance_history(&self.pool, worker_id, limit).await?;
        let run_id = entries.first().map(|entry| entry.run_id);
        Ok(crate::stores::WorkerSamplerPerformanceHistoryResponse { run_id, entries })
    }
}

#[async_trait::async_trait]
impl RuntimeLogStore for PgStore {
    async fn insert_runtime_log(&self, event: &RuntimeLogEvent) -> Result<(), StoreError> {
        queries::insert_runtime_log(&self.pool, event).await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl RunSpecStore for PgStore {
    async fn load_run_spec(&self, run_id: i32) -> Result<Option<RunSpec>, StoreError> {
        let Some((integration_params, point_spec)) =
            queries::load_run_spec_payload(&self.pool, run_id).await?
        else {
            return Ok(None);
        };

        let spec = decode_run_spec(run_id, integration_params, point_spec)?;
        Ok(Some(spec))
    }
}

#[async_trait::async_trait]
impl WorkerRegistryStore for PgStore {
    async fn register_worker(&self, worker: &Worker) -> Result<(), StoreError> {
        queries::register_worker(&self.pool, worker)
            .await
            .map_err(map_sqlx)
    }

    async fn heartbeat_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        queries::heartbeat_worker(&self.pool, worker_id)
            .await
            .map_err(map_sqlx)
    }

    async fn update_worker_status(
        &self,
        worker_id: &str,
        worker_status: WorkerStatus,
    ) -> Result<(), StoreError> {
        queries::update_worker_status(&self.pool, worker_id, worker_status)
            .await
            .map_err(map_sqlx)
    }

    async fn get_worker(&self, worker_id: &str) -> Result<Option<Worker>, StoreError> {
        let Some(row) = queries::get_worker(&self.pool, worker_id)
            .await
            .map_err(map_sqlx)?
        else {
            return Ok(None);
        };

        Ok(Some(Worker {
            worker_id: row.worker_id,
            node_id: row.node_id,
            role: row.role.parse().map_err(store_err)?,
            implementation: row.implementation,
            version: row.version,
            node_specs: row.node_specs,
            status: row.status.parse().map_err(store_err)?,
            last_seen: row.last_seen,
        }))
    }
}

#[async_trait::async_trait]
impl AssignmentLeaseStore for PgStore {
    async fn acquire_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError> {
        queries::acquire_sampler_aggregator_lease(&self.pool, run_id, worker_id, ttl)
            .await
            .map_err(map_sqlx)
    }

    async fn renew_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError> {
        queries::renew_sampler_aggregator_lease(&self.pool, run_id, worker_id, ttl)
            .await
            .map_err(map_sqlx)
    }

    async fn release_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
    ) -> Result<(), StoreError> {
        queries::release_sampler_aggregator_lease(&self.pool, run_id, worker_id)
            .await
            .map_err(map_sqlx)
    }

    async fn assign_evaluator(&self, run_id: i32, worker_id: &str) -> Result<(), StoreError> {
        queries::assign_evaluator(&self.pool, run_id, worker_id)
            .await
            .map_err(map_sqlx)
    }

    async fn unassign_evaluator(&self, run_id: i32, worker_id: &str) -> Result<(), StoreError> {
        queries::unassign_evaluator(&self.pool, run_id, worker_id)
            .await
            .map_err(map_sqlx)
    }

    async fn list_assigned_evaluators(&self, run_id: i32) -> Result<Vec<String>, StoreError> {
        queries::list_assigned_evaluators(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }
}

#[async_trait::async_trait]
impl ControlPlaneStore for PgStore {
    async fn upsert_desired_assignment(
        &self,
        node_id: &str,
        role: WorkerRole,
        run_id: i32,
    ) -> Result<(), StoreError> {
        queries::upsert_desired_assignment(&self.pool, node_id, role, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn clear_desired_assignment(
        &self,
        node_id: &str,
        role: WorkerRole,
    ) -> Result<(), StoreError> {
        queries::clear_desired_assignment(&self.pool, node_id, role)
            .await
            .map_err(map_sqlx)
    }

    async fn clear_desired_assignments_for_run(&self, run_id: i32) -> Result<u64, StoreError> {
        queries::clear_desired_assignments_for_run(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn clear_all_desired_assignments(&self) -> Result<u64, StoreError> {
        queries::clear_all_desired_assignments(&self.pool)
            .await
            .map_err(map_sqlx)
    }

    async fn get_desired_assignment(
        &self,
        node_id: &str,
        role: WorkerRole,
    ) -> Result<Option<DesiredAssignment>, StoreError> {
        let run_id = queries::get_desired_assignment_run_id(&self.pool, node_id, role)
            .await
            .map_err(map_sqlx)?;
        Ok(run_id.map(|run_id| DesiredAssignment {
            node_id: node_id.to_string(),
            role,
            run_id,
        }))
    }

    async fn list_desired_assignments(
        &self,
        node_id: Option<&str>,
    ) -> Result<Vec<DesiredAssignment>, StoreError> {
        let rows = queries::list_desired_assignments(&self.pool, node_id)
            .await
            .map_err(map_sqlx)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(DesiredAssignment {
                node_id: row.node_id,
                role: row.role.parse().map_err(store_err)?,
                run_id: row.run_id,
            });
        }
        Ok(out)
    }

    async fn request_node_shutdown(&self, node_id: &str) -> Result<u64, StoreError> {
        queries::request_node_shutdown(&self.pool, node_id)
            .await
            .map_err(map_sqlx)
    }

    async fn request_all_nodes_shutdown(&self) -> Result<u64, StoreError> {
        queries::request_all_nodes_shutdown(&self.pool)
            .await
            .map_err(map_sqlx)
    }

    async fn consume_node_shutdown_request(&self, node_id: &str) -> Result<bool, StoreError> {
        queries::consume_node_shutdown_request(&self.pool, node_id)
            .await
            .map_err(map_sqlx)
    }

    async fn create_run(
        &self,
        name: &str,
        integration_params: &JsonValue,
        target: Option<&JsonValue>,
        point_spec: &PointSpec,
        evaluator_init_metadata: Option<&JsonValue>,
        sampler_aggregator_init_metadata: Option<&JsonValue>,
    ) -> Result<i32, StoreError> {
        let sanitized_params = parse_run_create_payload(integration_params)?;

        queries::create_run(
            &self.pool,
            name,
            &sanitized_params,
            target,
            point_spec,
            evaluator_init_metadata,
            sampler_aggregator_init_metadata,
        )
        .await
        .map_err(map_sqlx)
    }

    async fn remove_run(&self, run_id: i32) -> Result<(), StoreError> {
        let rows = queries::remove_run(&self.pool, run_id)
            .await
            .map_err(map_sqlx)?;
        if rows == 0 {
            return Err(store_err(format!("run {run_id} not found")));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl WorkQueueStore for PgStore {
    async fn insert_batch(
        &self,
        run_id: i32,
        batch: &Batch,
        requires_training: bool,
    ) -> Result<i64, StoreError> {
        queries::insert_batch(&self.pool, run_id, batch, requires_training)
            .await
            .map_err(map_sqlx)
    }

    async fn get_pending_batch_count(&self, run_id: i32) -> Result<i64, StoreError> {
        queries::get_pending_batch_count(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn claim_batch(
        &self,
        run_id: i32,
        worker_id: &str,
    ) -> Result<Option<BatchClaim>, StoreError> {
        let claimed = queries::claim_batch(&self.pool, run_id, worker_id)
            .await
            .map_err(map_sqlx)?;

        Ok(
            claimed.map(|(batch_id, batch, requires_training)| BatchClaim {
                batch_id,
                batch,
                requires_training,
            }),
        )
    }

    async fn release_claimed_batches_for_worker(
        &self,
        run_id: i32,
        worker_id: &str,
    ) -> Result<u64, StoreError> {
        queries::release_claimed_batches_for_worker(&self.pool, run_id, worker_id)
            .await
            .map_err(map_sqlx)
    }

    async fn submit_batch_results(
        &self,
        batch_id: i64,
        result: &BatchResult,
        eval_time_ms: f64,
    ) -> Result<(), StoreError> {
        queries::submit_batch_results(&self.pool, batch_id, result, eval_time_ms)
            .await
            .map_err(map_sqlx)
    }

    async fn record_evaluator_performance_snapshot(
        &self,
        snapshot: &EvaluatorPerformanceSnapshot,
    ) -> Result<(), StoreError> {
        queries::insert_evaluator_performance_snapshot(&self.pool, snapshot)
            .await
            .map_err(map_sqlx)
    }

    async fn record_sampler_performance_snapshot(
        &self,
        snapshot: &SamplerAggregatorPerformanceSnapshot,
    ) -> Result<(), StoreError> {
        queries::insert_sampler_aggregator_performance_snapshot(&self.pool, snapshot)
            .await
            .map_err(map_sqlx)
    }

    async fn fail_batch(&self, batch_id: i64, last_error: &str) -> Result<(), StoreError> {
        queries::fail_batch(&self.pool, batch_id, last_error)
            .await
            .map_err(map_sqlx)
    }

    async fn fetch_completed_batches(
        &self,
        run_id: i32,
        limit: usize,
    ) -> Result<Vec<CompletedBatch>, StoreError> {
        let rows = queries::fetch_completed_batches(&self.pool, run_id, limit)
            .await
            .map_err(map_sqlx)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let batch = Batch::from_json(&row.points).map_err(|err| {
                store_err(format!(
                    "failed to deserialize batch points for batch_id={}: {err}",
                    row.batch_id
                ))
            })?;
            let result = BatchResult::values_from_json(row.values.as_ref(), &row.batch_observable)
                .map_err(|err| {
                    store_err(format!(
                        "failed to deserialize batch result payload for batch_id={}: {err}",
                        row.batch_id
                    ))
                })?;

            out.push(CompletedBatch {
                batch_id: row.batch_id,
                batch,
                requires_training: row.requires_training,
                result,
                completed_at: row.completed_at,
                total_eval_time_ms: row.total_eval_time_ms,
            });
        }

        Ok(out)
    }
    async fn try_set_training_completed_at(&self, run_id: i32) -> Result<bool, StoreError> {
        let rows = queries::try_set_training_completed_at(&self.pool, run_id)
            .await
            .map_err(map_sqlx)?;
        Ok(rows > 0)
    }
    async fn delete_completed_batches(&self, batch_ids: &[i64]) -> Result<(), StoreError> {
        queries::delete_completed_batches(&self.pool, batch_ids)
            .await
            .map_err(map_sqlx)
    }
}

#[async_trait::async_trait]
impl AggregationStore for PgStore {
    async fn load_current_observable(&self, run_id: i32) -> Result<Option<JsonValue>, StoreError> {
        queries::get_run_current_observable(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn load_sampler_runner_snapshot(
        &self,
        run_id: i32,
    ) -> Result<Option<JsonValue>, StoreError> {
        queries::get_run_sampler_runner_snapshot(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn load_latest_aggregation_snapshot(
        &self,
        run_id: i32,
    ) -> Result<Option<JsonValue>, StoreError> {
        queries::get_latest_aggregation_snapshot(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn save_aggregation(
        &self,
        run_id: i32,
        current_observable: &JsonValue,
        aggregated_observable: &JsonValue,
        delta_batches_completed: i32,
    ) -> Result<(), StoreError> {
        if delta_batches_completed <= 0 {
            return Ok(());
        }

        queries::insert_aggregated_results_snapshot(&self.pool, run_id, aggregated_observable)
            .await
            .map_err(map_sqlx)?;
        queries::update_run_aggregation(
            &self.pool,
            run_id,
            current_observable,
            delta_batches_completed,
        )
        .await
        .map_err(map_sqlx)?;

        Ok(())
    }

    async fn save_sampler_runner_snapshot(
        &self,
        run_id: i32,
        snapshot: &JsonValue,
    ) -> Result<(), StoreError> {
        queries::update_run_sampler_runner_snapshot(&self.pool, run_id, snapshot)
            .await
            .map_err(map_sqlx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
    use serde::Deserialize;
    use serde_json::json;

    #[test]
    fn decode_run_spec_supports_current_schema() {
        let spec = decode_run_spec(
            7,
            json!({
                "evaluator": { "kind": "sin_evaluator", "alpha": 1 },
                "sampler_aggregator": { "kind": "naive_monte_carlo", "beta": 2 },
                "parametrization": { "kind": "identity", "delta": 4 },
                "evaluator_runner_params": {
                    "min_loop_time_ms": 42,
                    "performance_snapshot_interval_ms": 5000
                },
                "sampler_aggregator_runner_params": {
                    "min_poll_time_ms": 500,
                    "performance_snapshot_interval_ms": 5000,
                    "target_batch_eval_ms": 200.0,
                    "target_queue_remaining": 0.0,
                    "lease_ttl_ms": 5000,
                    "max_batch_size": 64,
                    "max_queue_size": 128,
                    "max_batches_per_tick": 1,
                    "completed_batch_fetch_limit": 512
                }
            }),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect("decode");

        assert_eq!(spec.run_id, 7);
        assert_eq!(spec.point_spec.continuous_dims, 1);
        assert_eq!(spec.point_spec.discrete_dims, 0);
        assert_eq!(spec.evaluator.kind_str(), "sin_evaluator");
        assert!(matches!(
            &spec.evaluator,
            crate::engines::EvaluatorConfig::SinEvaluator { params }
                if params.get("alpha") == Some(&json!(1))
        ));
        assert_eq!(spec.sampler_aggregator.kind_str(), "naive_monte_carlo");
        assert!(matches!(
            &spec.sampler_aggregator,
            crate::engines::SamplerAggregatorConfig::NaiveMonteCarlo { params }
                if params.get("beta") == Some(&json!(2))
        ));
        assert_eq!(spec.parametrization.kind_str(), "identity");
        assert!(matches!(
            &spec.parametrization,
            crate::engines::ParametrizationConfig::Identity { params }
                if params.get("delta") == Some(&json!(4))
        ));
        assert_eq!(
            spec.evaluator_runner_params,
            EvaluatorRunnerParams::deserialize(json!({
                "min_loop_time_ms": 42,
                "performance_snapshot_interval_ms": 5000
            }))
            .unwrap()
        );
        assert_eq!(
            spec.sampler_aggregator_runner_params,
            SamplerAggregatorRunnerParams::deserialize(json!({
                "min_poll_time_ms": 500,
                "performance_snapshot_interval_ms": 5000,
                "target_batch_eval_ms": 200.0,
                "target_queue_remaining": 0.0,
                "lease_ttl_ms": 5000,
                "max_batch_size": 64,
                "max_queue_size": 128,
                "max_batches_per_tick": 1,
                "completed_batch_fetch_limit": 512
            }))
            .unwrap()
        );
    }

    #[test]
    fn decode_run_spec_requires_non_implementation_fields() {
        let err = decode_run_spec(
            8,
            json!({
                "evaluator": { "kind": "sin_evaluator" },
                "sampler_aggregator": { "kind": "naive_monte_carlo" },
                "parametrization": { "kind": "identity" }
            }),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect_err("missing params should fail");
        assert!(err.to_string().contains("evaluator_runner_params"));
    }

    #[test]
    fn decode_run_spec_requires_implementation_fields() {
        let err = decode_run_spec(
            9,
            json!({ "evaluator": { "kind": "sin_evaluator" } }),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect_err("missing required components should fail");
        assert!(err.to_string().contains("sampler_aggregator"));

        let err = decode_run_spec(
            9,
            json!({
                "evaluator": { "kind": "sin_evaluator" },
                "sampler_aggregator": { "kind": "naive_monte_carlo" }
            }),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect_err("missing parametrization implementation should fail");
        assert!(err.to_string().contains("parametrization"));
    }

    #[test]
    fn decode_run_spec_rejects_non_object_payload() {
        let err = decode_run_spec(
            10,
            json!("invalid-shape"),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect_err("non-object payload should fail");
        assert!(err.to_string().contains("expected object"));
    }

    #[test]
    fn parse_run_create_payload_accepts_kind_model() {
        let sanitized = parse_run_create_payload(&json!({
            "evaluator": { "kind": "sin_evaluator" },
            "sampler_aggregator": { "kind": "naive_monte_carlo" },
            "parametrization": { "kind": "identity", "a": 1 }
        }))
        .expect("parse");

        assert_eq!(
            sanitized,
            json!({
                "evaluator": { "kind": "sin_evaluator" },
                "sampler_aggregator": { "kind": "naive_monte_carlo" },
                "parametrization": { "kind": "identity", "a": 1 }
            })
        );
    }
}
