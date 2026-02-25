//! Store contracts for DB-backed control-plane, queue, and lifecycle APIs.

use super::errors::StoreError;
use super::models::{
    BatchClaim, CompletedBatch, DesiredAssignment, EvaluatorPerformanceSnapshot,
    SamplerAggregatorPerformanceSnapshot, Worker, WorkerStatus,
};
use crate::batch::{Batch, BatchResult, PointSpec};
use crate::engines::RunSpec;
use async_trait::async_trait;
use serde_json::Value as JsonValue;
use std::time::Duration;

/// Loads immutable run configuration.
#[async_trait]
pub trait RunSpecStore: Send + Sync {
    async fn load_run_spec(&self, run_id: i32) -> Result<Option<RunSpec>, StoreError>;
}

/// Registers and monitors running workers.
#[async_trait]
pub trait WorkerRegistryStore: Send + Sync {
    async fn register_worker(&self, worker: &Worker) -> Result<(), StoreError>;
    async fn heartbeat_worker(&self, worker_id: &str) -> Result<(), StoreError>;
    async fn update_worker_status(
        &self,
        worker_id: &str,
        status: WorkerStatus,
    ) -> Result<(), StoreError>;
    async fn get_worker(&self, worker_id: &str) -> Result<Option<Worker>, StoreError>;
}

/// Handles run assignment and lease ownership.
#[async_trait]
pub trait AssignmentLeaseStore: Send + Sync {
    async fn acquire_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError>;
    async fn renew_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError>;
    async fn release_sampler_aggregator_lease(
        &self,
        run_id: i32,
        worker_id: &str,
    ) -> Result<(), StoreError>;
    async fn assign_evaluator(&self, run_id: i32, worker_id: &str) -> Result<(), StoreError>;
    async fn unassign_evaluator(&self, run_id: i32, worker_id: &str) -> Result<(), StoreError>;
    async fn list_assigned_evaluators(&self, run_id: i32) -> Result<Vec<String>, StoreError>;
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
    async fn clear_desired_assignment(
        &self,
        node_id: &str,
        role: super::models::WorkerRole,
    ) -> Result<(), StoreError>;
    async fn get_desired_assignment(
        &self,
        node_id: &str,
        role: super::models::WorkerRole,
    ) -> Result<Option<DesiredAssignment>, StoreError>;
    async fn list_desired_assignments(
        &self,
        node_id: Option<&str>,
    ) -> Result<Vec<DesiredAssignment>, StoreError>;

    async fn create_run(
        &self,
        status: super::models::RunStatus,
        name: &str,
        integration_params: &JsonValue,
        point_spec: &PointSpec,
    ) -> Result<i32, StoreError>;
    async fn set_run_status(
        &self,
        run_id: i32,
        status: super::models::RunStatus,
    ) -> Result<(), StoreError>;
    async fn remove_run(&self, run_id: i32) -> Result<(), StoreError>;
}

/// Accesses the batch work queue.
#[async_trait]
pub trait WorkQueueStore: Send + Sync {
    async fn insert_batch(&self, run_id: i32, batch: &Batch) -> Result<i64, StoreError>;
    async fn get_pending_batch_count(&self, run_id: i32) -> Result<i64, StoreError>;
    async fn claim_batch(
        &self,
        run_id: i32,
        worker_id: &str,
    ) -> Result<Option<BatchClaim>, StoreError>;
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
    async fn delete_completed_batches(&self, batch_ids: &[i64]) -> Result<(), StoreError>;
}

/// Persists aggregated observable snapshots.
#[async_trait]
pub trait AggregationStore: Send + Sync {
    async fn load_latest_aggregation_snapshot(
        &self,
        run_id: i32,
    ) -> Result<Option<JsonValue>, StoreError>;
    async fn save_aggregation_snapshot(
        &self,
        run_id: i32,
        aggregated_observable: &JsonValue,
        delta_batches_completed: i32,
    ) -> Result<(), StoreError>;
}
