//! Store contracts for DB-backed control-plane, queue, and lifecycle APIs.

use super::errors::StoreError;
use super::models::{
    BatchClaim, CompletedBatch, DesiredAssignment, EvaluatorPerformanceSnapshot, RegisteredNode,
    RuntimeLogEvent, SamplerAggregatorPerformanceSnapshot,
};
use crate::core::{Batch, BatchResult, PointSpec};
use crate::engines::RunSpec;
use crate::stores::read_models::{
    AggregatedRangeResponse, AggregatedResult, EvaluatorPerformanceHistoryEntry,
    RegisteredWorkerEntry, RunProgress, SamplerPerformanceHistoryEntry, WorkQueueStats,
    WorkerEvaluatorPerformanceHistoryResponse, WorkerLogPage,
    WorkerSamplerPerformanceHistoryResponse,
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
        node_id: &str,
        role: super::models::WorkerRole,
        run_id: i32,
    ) -> Result<(), StoreError>;
    async fn register_node(&self, node_id: &str) -> Result<(), StoreError>;
    async fn heartbeat_node(&self, node_id: &str) -> Result<(), StoreError>;
    async fn set_current_assignment(
        &self,
        node_id: &str,
        role: super::models::WorkerRole,
        run_id: i32,
    ) -> Result<(), StoreError>;
    async fn clear_current_assignment(&self, node_id: &str) -> Result<(), StoreError>;
    async fn clear_desired_assignment(&self, node_id: &str) -> Result<(), StoreError>;
    async fn clear_desired_assignments_for_run(&self, run_id: i32) -> Result<u64, StoreError>;
    async fn clear_all_desired_assignments(&self) -> Result<u64, StoreError>;
    async fn get_desired_assignment(
        &self,
        node_id: &str,
    ) -> Result<Option<DesiredAssignment>, StoreError>;
    async fn list_desired_assignments(
        &self,
        node_id: Option<&str>,
    ) -> Result<Vec<DesiredAssignment>, StoreError>;
    async fn list_nodes(&self, node_id: Option<&str>) -> Result<Vec<RegisteredNode>, StoreError>;
    async fn request_node_shutdown(&self, node_id: &str) -> Result<u64, StoreError>;
    async fn request_all_nodes_shutdown(&self) -> Result<u64, StoreError>;
    async fn consume_node_shutdown_request(&self, node_id: &str) -> Result<bool, StoreError>;

    async fn create_run(
        &self,
        name: &str,
        integration_params: &JsonValue,
        target: Option<&JsonValue>,
        point_spec: &PointSpec,
        evaluator_init_metadata: Option<&JsonValue>,
        sampler_aggregator_init_metadata: Option<&JsonValue>,
    ) -> Result<i32, StoreError>;
    async fn remove_run(&self, run_id: i32) -> Result<(), StoreError>;
}

/// Accesses the batch work queue.
#[async_trait]
pub trait WorkQueueStore: Send + Sync {
    async fn insert_batch(
        &self,
        run_id: i32,
        batch: &Batch,
        requires_training: bool,
    ) -> Result<i64, StoreError>;
    async fn get_pending_batch_count(&self, run_id: i32) -> Result<i64, StoreError>;
    async fn claim_batch(
        &self,
        run_id: i32,
        node_id: &str,
    ) -> Result<Option<BatchClaim>, StoreError>;
    async fn release_claimed_batches_for_worker(
        &self,
        run_id: i32,
        node_id: &str,
    ) -> Result<u64, StoreError>;
    async fn submit_batch_results(
        &self,
        batch_id: i64,
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
    async fn try_set_training_completed_at(&self, run_id: i32) -> Result<bool, StoreError>;
    async fn delete_completed_batches(&self, batch_ids: &[i64]) -> Result<(), StoreError>;
}

/// Persists aggregated observable snapshots.
#[async_trait]
pub trait AggregationStore: Send + Sync {
    async fn load_current_observable(&self, run_id: i32) -> Result<Option<JsonValue>, StoreError>;
    async fn load_sampler_runner_snapshot(
        &self,
        run_id: i32,
    ) -> Result<Option<JsonValue>, StoreError>;
    async fn load_latest_aggregation_snapshot(
        &self,
        run_id: i32,
    ) -> Result<Option<JsonValue>, StoreError>;
    async fn save_aggregation(
        &self,
        run_id: i32,
        current_observable: &JsonValue,
        aggregated_observable: &JsonValue,
        delta_batches_completed: i32,
    ) -> Result<(), StoreError>;
    async fn save_sampler_runner_snapshot(
        &self,
        run_id: i32,
        snapshot: &JsonValue,
    ) -> Result<(), StoreError>;
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
    async fn get_latest_aggregated_result(
        &self,
        run_id: i32,
    ) -> Result<Option<AggregatedResult>, StoreError>;
    async fn get_aggregated_results(
        &self,
        run_id: i32,
        limit: i64,
    ) -> Result<Vec<AggregatedResult>, StoreError>;
    async fn get_aggregated_range(
        &self,
        run_id: i32,
        start: i64,
        stop: i64,
        max_points: i64,
        last_id: Option<i64>,
    ) -> Result<AggregatedRangeResponse, StoreError>;
    async fn get_worker_logs(
        &self,
        run_id: i32,
        limit: i64,
        worker_id: Option<&str>,
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
    ) -> Result<WorkerEvaluatorPerformanceHistoryResponse, StoreError>;
    async fn get_worker_sampler_performance_history(
        &self,
        worker_id: &str,
        limit: i64,
    ) -> Result<WorkerSamplerPerformanceHistoryResponse, StoreError>;
}
