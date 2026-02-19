//! Postgres-backed implementations of store contracts.

use super::queries;
use crate::batch::{Batch, BatchResults, PointSpec};
use crate::core::{
    AggregationStore, AssignmentLeaseStore, BatchClaim, CompletedBatch, ControlPlaneStore,
    DesiredAssignment, EngineStateStore, RunSpecStore, RunStatus, StoreError, WorkQueueStore,
    Worker, WorkerRegistryStore, WorkerRole, WorkerStatus,
};
use crate::engines::{EngineState, IntegrationParams, RunSpec};
use crate::stores::RunReadStore;
use serde_json::{Value as JsonValue, json};
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

fn map_sqlx(err: sqlx::Error) -> StoreError {
    store_err(err.to_string())
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
    let sampler_aggregator_implementation =
        params.sampler_aggregator_implementation.ok_or_else(|| {
            store_err(format!(
                "missing sampler_aggregator_implementation in integration_params for run_id={run_id}"
            ))
        })?;
    let observable_implementation = params.observable_implementation.ok_or_else(|| {
        store_err(format!(
            "missing observable_implementation in integration_params for run_id={run_id}"
        ))
    })?;

    Ok(RunSpec {
        run_id,
        point_spec,
        evaluator_implementation,
        evaluator_params: params.evaluator_params.unwrap_or_else(|| json!({})),
        sampler_aggregator_implementation,
        sampler_aggregator_params: params
            .sampler_aggregator_params
            .unwrap_or_else(|| json!({})),
        observable_implementation,
        observable_params: params.observable_params.unwrap_or_else(|| json!({})),
        worker_runner_params: params.worker_runner_params.unwrap_or_else(|| json!({})),
        sampler_aggregator_runner_params: params
            .sampler_aggregator_runner_params
            .unwrap_or_else(|| json!({})),
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

#[async_trait::async_trait]
impl RunReadStore for PgStore {
    async fn health_check(&self) -> Result<(), StoreError> {
        queries::health_check(&self.pool).await.map_err(map_sqlx)
    }

    async fn get_all_runs(&self) -> Result<Vec<crate::stores::RunProgress>, StoreError> {
        queries::get_all_runs(&self.pool).await.map_err(map_sqlx)
    }

    async fn get_run_progress(
        &self,
        run_id: i32,
    ) -> Result<Option<crate::stores::RunProgress>, StoreError> {
        queries::get_run_progress(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn get_work_queue_stats(
        &self,
        run_id: i32,
    ) -> Result<Vec<crate::stores::WorkQueueStats>, StoreError> {
        queries::get_work_queue_stats(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn get_latest_aggregated_result(
        &self,
        run_id: i32,
    ) -> Result<Option<crate::stores::AggregatedResult>, StoreError> {
        queries::get_latest_aggregated_result(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn get_aggregated_results(
        &self,
        run_id: i32,
        limit: i64,
    ) -> Result<Vec<crate::stores::AggregatedResult>, StoreError> {
        queries::get_aggregated_results(&self.pool, run_id, limit)
            .await
            .map_err(map_sqlx)
    }
}

#[async_trait::async_trait]
impl RunSpecStore for PgStore {
    async fn load_run_spec(&self, run_id: i32) -> Result<Option<RunSpec>, StoreError> {
        let Some((integration_params, point_spec)) =
            queries::load_run_spec_payload(&self.pool, run_id)
                .await
                .map_err(map_sqlx)?
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

    async fn create_run(
        &self,
        status: RunStatus,
        integration_params: &JsonValue,
        point_spec: &PointSpec,
    ) -> Result<i32, StoreError> {
        queries::create_run(&self.pool, status, integration_params, point_spec)
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
    async fn insert_batch(&self, run_id: i32, batch: &Batch) -> Result<(), StoreError> {
        queries::insert_batch(&self.pool, run_id, batch)
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

        Ok(claimed.map(|(batch_id, batch)| BatchClaim { batch_id, batch }))
    }

    async fn submit_batch_results(
        &self,
        batch_id: i64,
        results: &BatchResults,
        batch_observable: &JsonValue,
        eval_time_ms: f64,
    ) -> Result<(), StoreError> {
        queries::submit_batch_results(
            &self.pool,
            batch_id,
            results,
            batch_observable,
            eval_time_ms,
        )
        .await
        .map_err(map_sqlx)
    }

    async fn fail_batch(&self, batch_id: i64, last_error: &str) -> Result<(), StoreError> {
        queries::fail_batch(&self.pool, batch_id, last_error)
            .await
            .map_err(map_sqlx)
    }

    async fn fetch_completed_batches_since(
        &self,
        run_id: i32,
        last_batch_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<CompletedBatch>, StoreError> {
        let rows = queries::fetch_completed_batches_since(&self.pool, run_id, last_batch_id, limit)
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
            let results = BatchResults::from_json(&row.training_weights).map_err(|err| {
                store_err(format!(
                    "failed to deserialize batch training weights for batch_id={}: {err}",
                    row.batch_id
                ))
            })?;

            out.push(CompletedBatch {
                batch_id: row.batch_id,
                batch,
                results,
                batch_observable: row.batch_observable,
                completed_at: row.completed_at,
            });
        }

        Ok(out)
    }
}

#[async_trait::async_trait]
impl EngineStateStore for PgStore {
    async fn load_engine_state(&self, run_id: i32) -> Result<Option<EngineState>, StoreError> {
        let Some(state_json) = queries::load_engine_state(&self.pool, run_id)
            .await
            .map_err(map_sqlx)?
        else {
            return Ok(None);
        };
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
        queries::save_engine_state(&self.pool, run_id, &payload)
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
    use crate::engines::{
        EvaluatorImplementation, ObservableImplementation, SamplerAggregatorImplementation,
    };
    use serde_json::json;

    #[test]
    fn decode_run_spec_supports_current_schema() {
        let spec = decode_run_spec(
            7,
            json!({
                "evaluator_implementation": "test_only_sin",
                "evaluator_params": { "alpha": 1 },
                "sampler_aggregator_implementation": "test_only_training",
                "sampler_aggregator_params": { "beta": 2 },
                "observable_implementation": "test_only",
                "observable_params": { "gamma": 3 },
                "worker_runner_params": { "min_loop_time_ms": 42 },
                "sampler_aggregator_runner_params": { "interval_ms": 500 }
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
            EvaluatorImplementation::TestOnlySin
        );
        assert_eq!(spec.evaluator_params, json!({ "alpha": 1 }));
        assert_eq!(
            spec.sampler_aggregator_implementation,
            SamplerAggregatorImplementation::TestOnlyTraining
        );
        assert_eq!(spec.sampler_aggregator_params, json!({ "beta": 2 }));
        assert_eq!(
            spec.observable_implementation,
            ObservableImplementation::TestOnly
        );
        assert_eq!(spec.observable_params, json!({ "gamma": 3 }));
        assert_eq!(spec.worker_runner_params, json!({ "min_loop_time_ms": 42 }));
        assert_eq!(
            spec.sampler_aggregator_runner_params,
            json!({ "interval_ms": 500 })
        );
    }

    #[test]
    fn decode_run_spec_defaults_optional_param_payloads() {
        let spec = decode_run_spec(
            8,
            json!({
                "evaluator_implementation": "test_only_sin",
                "sampler_aggregator_implementation": "test_only_training",
                "observable_implementation": "test_only"
            }),
            json!({
                "continuous_dims": 1,
                "discrete_dims": 0
            }),
        )
        .expect("decode");

        assert_eq!(spec.evaluator_params, json!({}));
        assert_eq!(spec.sampler_aggregator_params, json!({}));
        assert_eq!(spec.observable_params, json!({}));
        assert_eq!(spec.worker_runner_params, json!({}));
        assert_eq!(spec.sampler_aggregator_runner_params, json!({}));
    }

    #[test]
    fn decode_run_spec_requires_implementation_fields() {
        let err = decode_run_spec(
            9,
            json!({ "evaluator_implementation": "test_only_sin" }),
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
}
