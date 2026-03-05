//! Postgres-backed implementations of store contracts.

use super::queries;
use crate::batch::{Batch, BatchResult, PointSpec};
use crate::core::{
    AggregationStore, AssignmentLeaseStore, BatchClaim, CompletedBatch, ControlPlaneStore,
    DesiredAssignment, EvaluatorPerformanceSnapshot, RunInitMetadataStore, RunSpecStore, RunStatus,
    RuntimeLogEvent, RuntimeLogStore, SamplerAggregatorPerformanceSnapshot, StoreError,
    WorkQueueStore, Worker, WorkerRegistryStore, WorkerRole, WorkerStatus,
};
use crate::engines::{IntegrationParams, ObservableImplementation, RunSpec};
use crate::stores::RunReadStore;
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

fn missing_integration_param(run_id: i32, field: &str) -> StoreError {
    store_err(format!(
        "missing {field} in integration_params for run_id={run_id}"
    ))
}

const AGGREGATED_RANGE_ANCHOR: i64 = 1;
const AGGREGATED_RANGE_MAX_POINTS: usize = 100;

fn resolve_aggregated_range_index(raw: i64, latest_id: i64) -> Result<i64, StoreError> {
    if raw == 0 {
        return Err(store_invalid_input(
            "invalid aggregated range index 0; use >=1 or negative indices",
        ));
    }
    if raw > 0 {
        return Ok(raw);
    }
    Ok(latest_id + raw + 1)
}

fn run_spec_from_integration_params(
    run_id: i32,
    point_spec: PointSpec,
    params: IntegrationParams,
) -> Result<RunSpec, StoreError> {
    let evaluator_implementation = params.evaluator_implementation.ok_or_else(|| {
        store_err(format!(
            "missing evaluator_implementation in integration_params for run_id={run_id}"
        ))
    })?;
    let sampler_aggregator_implementation = params.sampler_aggregator_implementation.ok_or_else(|| {
            store_err(format!(
                "missing sampler_aggregator_implementation in integration_params for run_id={run_id}"
            ))
        })?;
    let observable_implementation = params.observable_implementation.ok_or_else(|| {
        store_err(format!(
            "missing observable_implementation in integration_params for run_id={run_id}"
        ))
    })?;
    let evaluator_params = params
        .evaluator_params
        .ok_or_else(|| missing_integration_param(run_id, "evaluator_params"))?;
    let sampler_aggregator_params = params
        .sampler_aggregator_params
        .ok_or_else(|| missing_integration_param(run_id, "sampler_aggregator_params"))?;
    let observable_params = params
        .observable_params
        .ok_or_else(|| missing_integration_param(run_id, "observable_params"))?;
    let parametrization_implementation = params
        .parametrization_implementation
        .ok_or_else(|| missing_integration_param(run_id, "parametrization_implementation"))?;
    let parametrization_params = params
        .parametrization_params
        .ok_or_else(|| missing_integration_param(run_id, "parametrization_params"))?;
    let evaluator_runner_params = params
        .evaluator_runner_params
        .ok_or_else(|| missing_integration_param(run_id, "evaluator_runner_params"))?;
    let sampler_aggregator_runner_params = params
        .sampler_aggregator_runner_params
        .ok_or_else(|| missing_integration_param(run_id, "sampler_aggregator_runner_params"))?;

    Ok(RunSpec {
        run_id,
        point_spec,
        evaluator_implementation,
        evaluator_params,
        sampler_aggregator_implementation,
        sampler_aggregator_params,
        observable_implementation,
        observable_params,
        parametrization_implementation,
        parametrization_params,
        evaluator_runner_params,
        sampler_aggregator_runner_params,
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

fn parse_run_create_payload(
    integration_params: &JsonValue,
) -> Result<(JsonValue, ObservableImplementation), StoreError> {
    let mut root = integration_params.as_object().cloned().ok_or_else(|| {
        store_err("run create payload must be an object (integration_params json)")
    })?;

    let observable_implementation = if let Some(value) = root.remove("observable_implementation") {
        serde_json::from_value::<ObservableImplementation>(value).map_err(|err| {
            store_err(format!(
                "invalid observable_implementation in integration_params: {err}"
            ))
        })?
    } else {
        return Err(store_err(
            "missing observable_implementation in integration_params",
        ));
    };

    Ok((JsonValue::Object(root), observable_implementation))
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
        step: i64,
        latest_id: Option<i64>,
    ) -> Result<crate::stores::AggregatedRangeResponse, StoreError> {
        if step < 1 {
            return Err(store_invalid_input("aggregated range step must be >= 1"));
        }

        let Some((min_id, max_id)) = queries::get_aggregated_id_bounds(&self.pool, run_id).await?
        else {
            return Ok(crate::stores::AggregatedRangeResponse {
                snapshots: Vec::new(),
                latest: None,
                meta: crate::stores::AggregatedRangeMeta {
                    resolved_start: None,
                    resolved_stop: None,
                    step,
                    anchor: AGGREGATED_RANGE_ANCHOR,
                    latest_id: None,
                    max_points: AGGREGATED_RANGE_MAX_POINTS,
                },
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

        let estimated_points = ((resolved_stop - resolved_start) / step + 1).max(0) as usize;
        if estimated_points > AGGREGATED_RANGE_MAX_POINTS {
            return Err(store_invalid_input(format!(
                "aggregated range request exceeds max points: estimated={} max={} (increase step or shrink range)",
                estimated_points, AGGREGATED_RANGE_MAX_POINTS
            )));
        }

        let snapshots = queries::get_aggregated_range(
            &self.pool,
            run_id,
            resolved_start,
            resolved_stop,
            step,
            AGGREGATED_RANGE_ANCHOR,
            latest_id,
            AGGREGATED_RANGE_MAX_POINTS as i64,
        )
        .await?;

        let latest = queries::get_latest_aggregated_result(&self.pool, run_id).await?;

        Ok(crate::stores::AggregatedRangeResponse {
            snapshots,
            latest: latest.filter(|row| {
                latest_id.is_none_or(|seen| {
                    row.id
                        .parse::<i64>()
                        .ok()
                        .is_some_and(|latest_row_id| latest_row_id > seen)
                })
            }),
            meta: crate::stores::AggregatedRangeMeta {
                resolved_start: Some(resolved_start),
                resolved_stop: Some(resolved_stop),
                step,
                anchor: AGGREGATED_RANGE_ANCHOR,
                latest_id: Some(max_id.to_string()),
                max_points: AGGREGATED_RANGE_MAX_POINTS,
            },
        })
    }

    async fn get_worker_logs(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
        level: Option<&str>,
        after_id: Option<i64>,
    ) -> Result<Vec<crate::stores::WorkerLogEntry>, StoreError> {
        Ok(queries::get_worker_logs(&self.pool, run_id, limit, worker_id, level, after_id).await?)
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
impl RunInitMetadataStore for PgStore {
    async fn try_set_evaluator_init_metadata(
        &self,
        run_id: i32,
        metadata: &JsonValue,
    ) -> Result<bool, StoreError> {
        let rows = queries::try_set_evaluator_init_metadata(&self.pool, run_id, metadata)
            .await
            .map_err(map_sqlx)?;
        Ok(rows > 0)
    }

    async fn try_set_sampler_init_metadata(
        &self,
        run_id: i32,
        metadata: &JsonValue,
    ) -> Result<bool, StoreError> {
        let rows = queries::try_set_sampler_init_metadata(&self.pool, run_id, metadata)
            .await
            .map_err(map_sqlx)?;
        Ok(rows > 0)
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
        status: RunStatus,
        name: &str,
        integration_params: &JsonValue,
        target: Option<&JsonValue>,
        point_spec: &PointSpec,
    ) -> Result<i32, StoreError> {
        let (sanitized_params, observable_implementation) =
            parse_run_create_payload(integration_params)?;

        queries::create_run(
            &self.pool,
            status,
            name,
            &sanitized_params,
            target,
            observable_implementation.as_ref(),
            point_spec,
        )
        .await
        .map_err(map_sqlx)
    }

    async fn set_run_status(&self, run_id: i32, status: RunStatus) -> Result<(), StoreError> {
        let rows = queries::set_run_status(&self.pool, run_id, status)
            .await
            .map_err(map_sqlx)?;
        if rows == 0 {
            return Err(store_err(format!("run {run_id} not found")));
        }
        Ok(())
    }

    async fn set_all_runs_status(&self, status: RunStatus) -> Result<u64, StoreError> {
        queries::set_all_runs_status(&self.pool, status)
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
    async fn load_latest_aggregation_snapshot(
        &self,
        run_id: i32,
    ) -> Result<Option<JsonValue>, StoreError> {
        queries::get_latest_aggregation_snapshot(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn save_aggregation_snapshot(
        &self,
        run_id: i32,
        aggregated_observable: &JsonValue,
        delta_batches_completed: i32,
    ) -> Result<(), StoreError> {
        if delta_batches_completed <= 0 {
            return Ok(());
        }

        queries::insert_aggregated_results_snapshot(&self.pool, run_id, aggregated_observable)
            .await
            .map_err(map_sqlx)?;
        queries::update_run_summary_from_snapshot(&self.pool, run_id, delta_batches_completed)
            .await
            .map_err(map_sqlx)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engines::{
            EvaluatorImplementation, ObservableImplementation, ParametrizationImplementation,
            SamplerAggregatorImplementation,
        },
        runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams},
    };
    use serde::Deserialize;
    use serde_json::json;

    #[test]
    fn decode_run_spec_supports_current_schema() {
        let spec = decode_run_spec(
            7,
            json!({
                "evaluator_implementation": "sin_evaluator",
                "evaluator_params": { "alpha": 1 },
                "sampler_aggregator_implementation": "naive_monte_carlo",
                "sampler_aggregator_params": { "beta": 2 },
                "observable_implementation": "scalar",
                "observable_params": { "gamma": 3 },
                "parametrization_implementation": "identity",
                "parametrization_params": { "delta": 4 },
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
        assert_eq!(
            spec.evaluator_implementation,
            EvaluatorImplementation::SinEvaluator
        );
        assert_eq!(spec.evaluator_params, json!({ "alpha": 1 }));
        assert_eq!(
            spec.sampler_aggregator_implementation,
            SamplerAggregatorImplementation::NaiveMonteCarlo
        );
        assert_eq!(spec.sampler_aggregator_params, json!({ "beta": 2 }));
        assert_eq!(
            spec.observable_implementation,
            ObservableImplementation::Scalar
        );
        assert_eq!(spec.observable_params, json!({ "gamma": 3 }));
        assert_eq!(
            spec.parametrization_implementation,
            ParametrizationImplementation::Identity
        );
        assert_eq!(spec.parametrization_params, json!({ "delta": 4 }));
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
                "evaluator_implementation": "sin_evaluator",
                "sampler_aggregator_implementation": "naive_monte_carlo",
                "observable_implementation": "scalar"
            }),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect_err("missing params should fail");
        assert!(err.to_string().contains("evaluator_params"));
    }

    #[test]
    fn decode_run_spec_requires_implementation_fields() {
        let err = decode_run_spec(
            9,
            json!({ "evaluator_implementation": "sin_evaluator" }),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect_err("missing required components should fail");
        assert!(
            err.to_string()
                .contains("sampler_aggregator_implementation")
        );

        let err = decode_run_spec(
            9,
            json!({
                "evaluator_implementation": "sin_evaluator",
                "sampler_aggregator_implementation": "naive_monte_carlo",
            }),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect_err("missing observable implementation should fail");
        assert!(err.to_string().contains("observable_implementation"));
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
    fn parse_run_create_payload_extracts_observable_implementation() {
        let (sanitized, observable) = parse_run_create_payload(&json!({
            "evaluator_implementation": "sin_evaluator",
            "sampler_aggregator_implementation": "naive_monte_carlo",
            "observable_implementation": "scalar",
            "observable_params": { "a": 1 }
        }))
        .expect("parse");

        assert_eq!(observable, ObservableImplementation::Scalar);
        assert_eq!(
            sanitized,
            json!({
                "evaluator_implementation": "sin_evaluator",
                "sampler_aggregator_implementation": "naive_monte_carlo",
                "observable_params": { "a": 1 }
            })
        );
    }

    #[test]
    fn parse_run_create_payload_requires_observable_implementation() {
        let err = parse_run_create_payload(&json!({
            "evaluator_implementation": "sin_evaluator"
        }))
        .expect_err("missing observable_implementation should fail");
        assert!(
            err.to_string()
                .contains("missing observable_implementation")
        );
    }

    #[test]
    fn parse_run_create_payload_rejects_invalid_observable() {
        let err = parse_run_create_payload(&json!({
            "observable_implementation": "does_not_exist"
        }))
        .expect_err("invalid observable should fail");
        assert!(
            err.to_string()
                .contains("invalid observable_implementation")
        );
    }
}
