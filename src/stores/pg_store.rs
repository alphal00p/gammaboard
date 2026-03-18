//! Postgres-backed implementations of store contracts.

use super::queries;
use crate::core::{
    AggregationStore, BatchClaim, CompletedBatch, ControlPlaneStore, DesiredAssignment,
    EvaluatorPerformanceSnapshot, ParametrizationState, ParametrizationVersionStore,
    RegisteredNode, RunReadStore, RunSampleProgress, RunSpecStore, RunStageSnapshot, RunTask,
    RunTaskSpec, RunTaskStore, RuntimeLogEvent, RuntimeLogStore,
    SamplerAggregatorPerformanceSnapshot, StoreError, WorkQueueStore, WorkerRole,
};
use crate::core::{IntegrationParams, RunSpec};
use crate::evaluation::{BatchResult, PointSpec};
use crate::sampling::LatentBatch;
use serde_json::Value as JsonValue;
use sqlx::PgPool;

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
    if let sqlx::Error::Database(db_err) = &err {
        if db_err.code().as_deref() == Some("23505") {
            if db_err.constraint() == Some("idx_nodes_desired_sampler_run") {
                return StoreError::invalid_input(
                    "run already has a sampler_aggregator assignment; clear the existing sampler assignment before assigning another node",
                );
            }
            if db_err.constraint() == Some("idx_nodes_current_sampler_run") {
                return StoreError::invalid_input(
                    "run already has a current sampler_aggregator node; clear the existing current sampler before starting another node",
                );
            }
        }
        if db_err.code().as_deref() == Some("23514") {
            if matches!(
                db_err.constraint(),
                Some("nodes_desired_assignment_pair_check")
                    | Some("nodes_current_assignment_pair_check")
            ) {
                return StoreError::invalid_input(
                    "node desired/current role and run fields must be both set or both null",
                );
            }
        }
    }
    StoreError::from(err)
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
        observable: params.observable,
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

    async fn get_task_output_snapshots(
        &self,
        run_id: i32,
        task_id: i64,
        after_snapshot_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<crate::stores::TaskOutputSnapshot>, StoreError> {
        Ok(queries::get_task_output_snapshots(
            &self.pool,
            run_id,
            task_id,
            after_snapshot_id,
            limit,
        )
        .await?)
    }

    async fn get_latest_task_stage_snapshot(
        &self,
        run_id: i32,
        task_id: i64,
    ) -> Result<Option<crate::stores::TaskStageSnapshot>, StoreError> {
        Ok(queries::get_latest_task_stage_snapshot(&self.pool, run_id, task_id).await?)
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
    ) -> Result<Vec<crate::stores::EvaluatorPerformanceHistoryEntry>, StoreError> {
        Ok(queries::get_worker_evaluator_performance_history(&self.pool, worker_id, limit).await?)
    }

    async fn get_worker_sampler_performance_history(
        &self,
        worker_id: &str,
        limit: i64,
    ) -> Result<Vec<crate::stores::SamplerPerformanceHistoryEntry>, StoreError> {
        Ok(queries::get_worker_sampler_performance_history(&self.pool, worker_id, limit).await?)
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

    async fn register_node(&self, node_id: &str) -> Result<(), StoreError> {
        queries::register_node(&self.pool, node_id)
            .await
            .map_err(map_sqlx)
    }

    async fn heartbeat_node(&self, node_id: &str) -> Result<(), StoreError> {
        queries::heartbeat_node(&self.pool, node_id)
            .await
            .map_err(map_sqlx)
    }

    async fn set_current_assignment(
        &self,
        node_id: &str,
        role: WorkerRole,
        run_id: i32,
    ) -> Result<(), StoreError> {
        queries::set_current_assignment(&self.pool, node_id, role, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn clear_current_assignment(&self, node_id: &str) -> Result<(), StoreError> {
        queries::clear_current_assignment(&self.pool, node_id)
            .await
            .map_err(map_sqlx)
    }

    async fn clear_desired_assignment(&self, node_id: &str) -> Result<(), StoreError> {
        queries::clear_desired_assignment(&self.pool, node_id)
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
    ) -> Result<Option<DesiredAssignment>, StoreError> {
        let assignment = queries::get_desired_assignment(&self.pool, node_id)
            .await
            .map_err(map_sqlx)?;
        assignment
            .map(|row| {
                Ok(DesiredAssignment {
                    node_id: row.node_id,
                    role: row.role.parse().map_err(store_err)?,
                    run_id: row.run_id,
                })
            })
            .transpose()
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

    async fn list_nodes(&self, node_id: Option<&str>) -> Result<Vec<RegisteredNode>, StoreError> {
        let rows = queries::list_nodes(&self.pool, node_id)
            .await
            .map_err(map_sqlx)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let desired_assignment = match (row.desired_role, row.desired_run_id) {
                (Some(role), Some(run_id)) => Some(DesiredAssignment {
                    node_id: row.node_id.clone(),
                    role: role.parse().map_err(store_err)?,
                    run_id,
                }),
                (None, None) => None,
                _ => return Err(store_err("invalid node assignment row")),
            };
            let current_assignment = match (row.current_role, row.current_run_id) {
                (Some(role), Some(run_id)) => Some(DesiredAssignment {
                    node_id: row.node_id.clone(),
                    role: role.parse().map_err(store_err)?,
                    run_id,
                }),
                (None, None) => None,
                _ => return Err(store_err("invalid current node assignment row")),
            };
            out.push(RegisteredNode {
                node_id: row.node_id,
                desired_assignment,
                current_assignment,
                last_seen: row.last_seen,
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
        initial_tasks: &[RunTaskSpec],
    ) -> Result<i32, StoreError> {
        let sanitized_params = parse_run_create_payload(integration_params)?;
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
        let run_id = sqlx::query_scalar(
            r#"
            INSERT INTO runs (
                name,
                integration_params,
                target,
                point_spec,
                evaluator_init_metadata,
                sampler_aggregator_init_metadata
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
        )
        .bind(name)
        .bind(&sanitized_params)
        .bind(target)
        .bind(sqlx::types::Json(point_spec))
        .bind(evaluator_init_metadata)
        .bind(sampler_aggregator_init_metadata)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        for (offset, task) in initial_tasks.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO run_tasks (
                    run_id,
                    sequence_nr,
                    task,
                    state
                )
                VALUES ($1, $2, $3, 'pending')
                "#,
            )
            .bind(run_id)
            .bind(offset as i32 + 1)
            .bind(serde_json::to_value(task).unwrap_or_default())
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        }
        tx.commit().await.map_err(map_sqlx)?;
        Ok(run_id)
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
        batch: &LatentBatch,
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

    async fn get_open_batch_count(&self, run_id: i32) -> Result<i64, StoreError> {
        queries::get_open_batch_count(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn claim_batch(
        &self,
        run_id: i32,
        node_id: &str,
    ) -> Result<Option<BatchClaim>, StoreError> {
        let claimed = queries::claim_batch(&self.pool, run_id, node_id)
            .await
            .map_err(map_sqlx)?;

        Ok(
            claimed.map(|(batch_id, latent_batch, requires_training)| BatchClaim {
                batch_id,
                latent_batch,
                requires_training,
            }),
        )
    }

    async fn release_claimed_batches_for_worker(
        &self,
        run_id: i32,
        node_id: &str,
    ) -> Result<u64, StoreError> {
        queries::release_claimed_batches_for_worker(&self.pool, run_id, node_id)
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
            let latent_batch = LatentBatch::from_json(&row.latent_batch).map_err(|err| {
                store_err(format!(
                    "failed to deserialize latent batch payload for batch_id={}: {err}",
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
                latent_batch,
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
impl ParametrizationVersionStore for PgStore {
    async fn load_parametrization_version(
        &self,
        run_id: i32,
        version: i64,
    ) -> Result<Option<ParametrizationState>, StoreError> {
        let row = queries::get_parametrization_state(&self.pool, run_id, version)
            .await
            .map_err(map_sqlx)?;
        row.map(|payload| {
            serde_json::from_value(payload)
                .map_err(|err| store_err(format!("failed to decode parametrization state: {err}")))
        })
        .transpose()
    }

    async fn load_latest_parametrization_version(
        &self,
        run_id: i32,
    ) -> Result<Option<i64>, StoreError> {
        queries::get_latest_parametrization_state_version(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn save_parametrization_version(
        &self,
        run_id: i32,
        version: i64,
        state: &ParametrizationState,
    ) -> Result<(), StoreError> {
        let payload = serde_json::to_value(state).map_err(|err| {
            store_err(format!("failed to serialize parametrization state: {err}"))
        })?;
        queries::upsert_parametrization_state(&self.pool, run_id, version, &payload)
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

    async fn load_run_sample_progress(
        &self,
        run_id: i32,
    ) -> Result<Option<RunSampleProgress>, StoreError> {
        let row = queries::get_run_sample_progress(&self.pool, run_id)
            .await
            .map_err(map_sqlx)?;
        Ok(row.map(
            |(nr_produced_samples, nr_completed_samples)| RunSampleProgress {
                nr_produced_samples,
                nr_completed_samples,
            },
        ))
    }

    async fn save_aggregation(
        &self,
        run_id: i32,
        task_id: i64,
        current_observable: &JsonValue,
        persisted_observable: &JsonValue,
        delta_batches_completed: i32,
    ) -> Result<(), StoreError> {
        if delta_batches_completed <= 0 {
            return Ok(());
        }

        queries::insert_persisted_observable_snapshot(
            &self.pool,
            run_id,
            task_id,
            persisted_observable,
        )
        .await
        .map_err(map_sqlx)?;
        queries::update_run_current_observable(
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

    async fn save_run_sample_progress(
        &self,
        run_id: i32,
        nr_produced_samples: i64,
        nr_completed_samples: i64,
    ) -> Result<(), StoreError> {
        queries::update_run_sample_progress(
            &self.pool,
            run_id,
            nr_produced_samples,
            nr_completed_samples,
        )
        .await
        .map_err(map_sqlx)
    }

    async fn save_run_stage_snapshot(&self, snapshot: &RunStageSnapshot) -> Result<(), StoreError> {
        queries::insert_run_stage_snapshot(&self.pool, snapshot)
            .await
            .map_err(map_sqlx)
    }
}

#[async_trait::async_trait]
impl RunTaskStore for PgStore {
    async fn append_run_tasks(
        &self,
        run_id: i32,
        tasks: &[RunTaskSpec],
    ) -> Result<Vec<RunTask>, StoreError> {
        queries::append_run_tasks(&self.pool, run_id, tasks)
            .await
            .map_err(map_sqlx)
    }

    async fn list_run_tasks(&self, run_id: i32) -> Result<Vec<RunTask>, StoreError> {
        queries::list_run_tasks(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn remove_pending_run_task(&self, run_id: i32, task_id: i64) -> Result<bool, StoreError> {
        queries::remove_pending_run_task(&self.pool, run_id, task_id)
            .await
            .map_err(map_sqlx)
    }

    async fn load_active_run_task(&self, run_id: i32) -> Result<Option<RunTask>, StoreError> {
        queries::load_active_run_task(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn activate_next_run_task(&self, run_id: i32) -> Result<Option<RunTask>, StoreError> {
        queries::activate_next_run_task(&self.pool, run_id)
            .await
            .map_err(map_sqlx)
    }

    async fn update_run_task_progress(
        &self,
        task_id: i64,
        nr_produced_samples: i64,
        nr_completed_samples: i64,
    ) -> Result<(), StoreError> {
        queries::update_run_task_progress(
            &self.pool,
            task_id,
            nr_produced_samples,
            nr_completed_samples,
        )
        .await
        .map_err(map_sqlx)
    }

    async fn complete_run_task(&self, task_id: i64) -> Result<(), StoreError> {
        queries::complete_run_task(&self.pool, task_id)
            .await
            .map_err(map_sqlx)
    }

    async fn fail_run_task(&self, task_id: i64, reason: &str) -> Result<(), StoreError> {
        queries::fail_run_task(&self.pool, task_id, reason)
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
                "evaluator": {
                    "kind": "sin_evaluator",
                    "min_eval_time_per_sample_ms": 1
                },
                "observable": "scalar",
                "sampler_aggregator": {
                    "kind": "naive_monte_carlo",
                    "training_target_samples": 2
                },
                "parametrization": { "kind": "identity" },
                "evaluator_runner_params": {
                    "min_loop_time_ms": 42,
                    "performance_snapshot_interval_ms": 5000
                },
                "sampler_aggregator_runner_params": {
                    "min_poll_time_ms": 500,
                    "performance_snapshot_interval_ms": 5000,
                    "target_batch_eval_ms": 200.0,
                    "target_queue_remaining": 0.0,
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
            crate::core::EvaluatorConfig::SinEvaluator { params }
                if params.min_eval_time_per_sample_ms == 1
        ));
        assert!(matches!(
            spec.observable,
            crate::core::ObservableConfig::Scalar
        ));
        assert_eq!(spec.sampler_aggregator.kind_str(), "naive_monte_carlo");
        assert!(matches!(
            &spec.sampler_aggregator,
            crate::core::SamplerAggregatorConfig::NaiveMonteCarlo { params }
                if params.training_target_samples == 2
        ));
        assert_eq!(spec.parametrization.kind_str(), "identity");
        assert!(matches!(
            &spec.parametrization,
            crate::core::ParametrizationConfig::Identity { .. }
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
                "observable": "scalar",
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
            json!({
                "evaluator": { "kind": "sin_evaluator" },
                "observable": "scalar"
            }),
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
                "observable": "scalar",
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
