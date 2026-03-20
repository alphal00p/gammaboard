//! Store contracts for DB-backed control-plane, queue, and lifecycle APIs.

use super::errors::StoreError;
use super::models::{
    BatchClaim, CompletedBatch, DesiredAssignment, EvaluatorPerformanceSnapshot, RegisteredNode,
    RunSampleProgress, RunStageSnapshot, RuntimeLogEvent, SamplerAggregatorPerformanceSnapshot,
};
use crate::core::RunSpec;
use crate::core::{RunTask, RunTaskSpec};
use crate::evaluation::{BatchResult, PointSpec};
use crate::runners::sampler_aggregator::SamplerAggregatorRunnerSnapshot;
use crate::sampling::LatentBatch;
use crate::stores::read_models::{
    EvaluatorPerformanceHistoryEntry, RegisteredWorkerEntry, RunProgress,
    SamplerPerformanceHistoryEntry, TaskOutputSnapshot, TaskStageSnapshot, WorkQueueStats,
    WorkerLogPage,
};
use async_trait::async_trait;
use serde_json::Value as JsonValue;

/// Loads immutable run configuration.
#[async_trait]
pub trait RunSpecStore: Send + Sync {
    async fn load_run_spec(&self, run_id: i32) -> Result<Option<RunSpec>, StoreError>;
}

/// Desired-state control-plane operations for node assignments and run steering.
#[async_trait]
pub trait ControlPlaneStore: Send + Sync {
    async fn upsert_desired_assignment(
        &self,
        node_name: &str,
        role: super::models::WorkerRole,
        run_id: i32,
    ) -> Result<(), StoreError>;
    async fn announce_node(&self, node_name: &str, node_uuid: &str) -> Result<(), StoreError>;
    async fn set_current_assignment(
        &self,
        node_uuid: &str,
        role: super::models::WorkerRole,
        run_id: i32,
    ) -> Result<(), StoreError>;
    async fn clear_current_assignment(&self, node_uuid: &str) -> Result<(), StoreError>;
    async fn clear_desired_assignment(&self, node_name: &str) -> Result<(), StoreError>;
    async fn clear_desired_assignments_for_run(&self, run_id: i32) -> Result<u64, StoreError>;
    async fn clear_all_desired_assignments(&self) -> Result<u64, StoreError>;
    async fn get_desired_assignment(
        &self,
        node_name: &str,
    ) -> Result<Option<DesiredAssignment>, StoreError>;
    async fn list_desired_assignments(
        &self,
        node_name: Option<&str>,
    ) -> Result<Vec<DesiredAssignment>, StoreError>;
    async fn list_nodes(&self, node_name: Option<&str>) -> Result<Vec<RegisteredNode>, StoreError>;
    async fn request_node_shutdown(&self, node_name: &str) -> Result<u64, StoreError>;
    async fn request_all_nodes_shutdown(&self) -> Result<u64, StoreError>;
    async fn consume_node_shutdown_request(&self, node_uuid: &str) -> Result<bool, StoreError>;
    async fn expire_node_lease(&self, node_uuid: &str) -> Result<(), StoreError>;

    async fn create_run(
        &self,
        name: &str,
        integration_params: &JsonValue,
        target: Option<&JsonValue>,
        point_spec: &PointSpec,
        evaluator_init_metadata: Option<&JsonValue>,
        sampler_aggregator_init_metadata: Option<&JsonValue>,
        initial_tasks: &[RunTaskSpec],
    ) -> Result<i32, StoreError>;
    async fn remove_run(&self, run_id: i32) -> Result<(), StoreError>;
}

/// Accesses the batch work queue.
#[async_trait]
pub trait WorkQueueStore: Send + Sync {
    async fn insert_batch(
        &self,
        run_id: i32,
        task_id: i64,
        batch: &LatentBatch,
    ) -> Result<i64, StoreError>;
    async fn get_pending_batch_count(&self, run_id: i32) -> Result<i64, StoreError>;
    async fn get_open_batch_count(&self, run_id: i32) -> Result<i64, StoreError>;
    async fn claim_batch(
        &self,
        run_id: i32,
        node_uuid: &str,
    ) -> Result<Option<BatchClaim>, StoreError>;
    async fn release_claimed_batches_for_worker(
        &self,
        run_id: i32,
        node_uuid: &str,
    ) -> Result<u64, StoreError>;
    async fn submit_batch_results(
        &self,
        batch_id: i64,
        node_uuid: &str,
        result: &BatchResult,
        eval_time_ms: f64,
    ) -> Result<(), StoreError>;
    async fn record_evaluator_performance_snapshot(
        &self,
        snapshot: &EvaluatorPerformanceSnapshot,
    ) -> Result<(), StoreError>;
    async fn record_sampler_performance_snapshot(
        &self,
        snapshot: &SamplerAggregatorPerformanceSnapshot,
    ) -> Result<(), StoreError>;
    async fn fail_batch(&self, batch_id: i64, last_error: &str) -> Result<(), StoreError>;
    async fn fetch_completed_batches(
        &self,
        run_id: i32,
        limit: usize,
    ) -> Result<Vec<CompletedBatch>, StoreError>;
    async fn delete_completed_batches(&self, batch_ids: &[i64]) -> Result<(), StoreError>;
    async fn reclaim_abandoned_batches(&self, run_id: i32) -> Result<u64, StoreError>;
}

/// Persists active-stage observable state and task-local persisted snapshots.
#[async_trait]
pub trait AggregationStore: Send + Sync {
    async fn load_current_observable(&self, run_id: i32) -> Result<Option<JsonValue>, StoreError>;
    async fn load_sampler_runner_snapshot(
        &self,
        run_id: i32,
    ) -> Result<Option<SamplerAggregatorRunnerSnapshot>, StoreError>;
    async fn load_latest_stage_snapshot_before_sequence(
        &self,
        run_id: i32,
        sequence_nr: i32,
    ) -> Result<Option<RunStageSnapshot>, StoreError>;
    async fn load_latest_stage_snapshot_for_task(
        &self,
        run_id: i32,
        task_id: i64,
    ) -> Result<Option<RunStageSnapshot>, StoreError>;
    async fn load_task_activation_snapshot(
        &self,
        run_id: i32,
        task_id: i64,
    ) -> Result<Option<RunStageSnapshot>, StoreError>;
    async fn load_run_sample_progress(
        &self,
        run_id: i32,
    ) -> Result<Option<RunSampleProgress>, StoreError>;
    async fn save_aggregation(
        &self,
        run_id: i32,
        task_id: i64,
        current_observable: &JsonValue,
        persisted_observable: &JsonValue,
        delta_batches_completed: i32,
    ) -> Result<(), StoreError>;
    async fn save_sampler_runner_snapshot(
        &self,
        run_id: i32,
        snapshot: &SamplerAggregatorRunnerSnapshot,
    ) -> Result<(), StoreError>;
    async fn save_run_sample_progress(
        &self,
        run_id: i32,
        nr_produced_samples: i64,
        nr_completed_samples: i64,
    ) -> Result<(), StoreError>;
    async fn save_run_stage_snapshot(&self, snapshot: &RunStageSnapshot) -> Result<(), StoreError>;
}

#[async_trait]
pub trait RunTaskStore: Send + Sync {
    async fn append_run_tasks(
        &self,
        run_id: i32,
        tasks: &[RunTaskSpec],
    ) -> Result<Vec<RunTask>, StoreError>;
    async fn list_run_tasks(&self, run_id: i32) -> Result<Vec<RunTask>, StoreError>;
    async fn remove_pending_run_task(&self, run_id: i32, task_id: i64) -> Result<bool, StoreError>;
    async fn load_active_run_task(&self, run_id: i32) -> Result<Option<RunTask>, StoreError>;
    async fn activate_next_run_task(&self, run_id: i32) -> Result<Option<RunTask>, StoreError>;
    async fn update_run_task_progress(
        &self,
        task_id: i64,
        nr_produced_samples: i64,
        nr_completed_samples: i64,
    ) -> Result<(), StoreError>;
    async fn set_run_task_spawn_origin(
        &self,
        task_id: i64,
        spawned_from_run_id: Option<i32>,
        spawned_from_task_id: Option<i64>,
    ) -> Result<(), StoreError>;
    async fn complete_run_task(&self, task_id: i64) -> Result<(), StoreError>;
    async fn fail_run_task(&self, task_id: i64, reason: &str) -> Result<(), StoreError>;
}

#[async_trait]
pub trait EvaluatorWorkerStore: WorkQueueStore + AggregationStore + Send + Sync {}

impl<T> EvaluatorWorkerStore for T where T: WorkQueueStore + AggregationStore + Send + Sync {}

#[async_trait]
pub trait SamplerWorkerStore:
    WorkQueueStore + AggregationStore + RunTaskStore + ControlPlaneStore + Send + Sync
{
}

impl<T> SamplerWorkerStore for T where
    T: WorkQueueStore + AggregationStore + RunTaskStore + ControlPlaneStore + Send + Sync
{
}

/// Persists runtime tracing events.
#[async_trait]
pub trait RuntimeLogStore: Send + Sync {
    async fn insert_runtime_log(&self, event: &RuntimeLogEvent) -> Result<(), StoreError>;
}

/// Read-only dashboard access.
#[async_trait]
pub trait RunReadStore: Send + Sync {
    async fn health_check(&self) -> Result<(), StoreError>;
    async fn get_all_runs(&self) -> Result<Vec<RunProgress>, StoreError>;
    async fn get_run_progress(&self, run_id: i32) -> Result<Option<RunProgress>, StoreError>;
    async fn get_work_queue_stats(&self, run_id: i32) -> Result<Vec<WorkQueueStats>, StoreError>;
    async fn get_task_output_snapshots(
        &self,
        run_id: i32,
        task_id: i64,
        after_snapshot_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<TaskOutputSnapshot>, StoreError>;
    async fn get_latest_task_stage_snapshot(
        &self,
        run_id: i32,
        task_id: i64,
    ) -> Result<Option<TaskStageSnapshot>, StoreError>;
    async fn get_worker_logs(
        &self,
        run_id: i32,
        limit: i64,
        node_name: Option<&str>,
        level: Option<&str>,
        query: Option<&str>,
        before_id: Option<i64>,
    ) -> Result<WorkerLogPage, StoreError>;
    async fn get_registered_workers(
        &self,
        run_id: Option<i32>,
    ) -> Result<Vec<RegisteredWorkerEntry>, StoreError>;
    async fn get_evaluator_performance_history(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
    ) -> Result<Vec<EvaluatorPerformanceHistoryEntry>, StoreError>;
    async fn get_sampler_performance_history(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
    ) -> Result<Vec<SamplerPerformanceHistoryEntry>, StoreError>;
    async fn get_worker_evaluator_performance_history(
        &self,
        worker_id: &str,
        limit: i64,
    ) -> Result<Vec<EvaluatorPerformanceHistoryEntry>, StoreError>;
    async fn get_worker_sampler_performance_history(
        &self,
        worker_id: &str,
        limit: i64,
    ) -> Result<Vec<SamplerPerformanceHistoryEntry>, StoreError>;
}
