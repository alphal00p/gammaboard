//! Core extension contracts for pluggable evaluators and sampler-aggregators.
//!
//! This module defines:
//! - Runtime traits (`Evaluator`, `SamplerAggregatorEngine`)
//! - Factory traits to build runtime objects from DB-backed run specs
//! - Store traits for DB-backed control-plane and queue access

#![allow(async_fn_in_trait)]

use super::errors::{BuildError, EngineError, EvalError, StoreError};
use super::models::{
    BatchClaim, CompletedBatch, ComponentInstance, EngineState, InstanceStatus, RunSpec,
};
use crate::models::{AggregatedResult, RunProgress, WorkQueueStats};
use crate::{Batch, BatchResults};
use serde_json::Value as JsonValue;
use std::time::Duration;

/// Evaluates integrand values for sample points.
pub trait Evaluator: Send + Sync {
    fn eval_point(&self, point: &JsonValue) -> Result<f64, EvalError>;

    fn eval_batch(&self, batch: &Batch) -> Result<BatchResults, EvalError> {
        let mut values = Vec::with_capacity(batch.size());
        for p in &batch.points {
            values.push(self.eval_point(&p.point)?);
        }
        Ok(BatchResults::new(values))
    }
}

/// Builds evaluator instances from a run specification.
pub trait EvaluatorFactory: Send + Sync {
    fn implementation(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn build(&self, spec: &RunSpec) -> Result<Box<dyn Evaluator>, BuildError>;
}

/// Owns adaptive sampling training and aggregation for a single run.
pub trait SamplerAggregatorEngine: Send {
    fn implementation(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn init(&mut self, state: Option<EngineState>) -> Result<(), EngineError>;
    fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<Batch>, EngineError>;
    fn ingest_completed(&mut self, completed: &[CompletedBatch]) -> Result<(), EngineError>;
}

/// Builds sampler-aggregator engines from a run specification.
pub trait SamplerAggregatorFactory: Send + Sync {
    fn implementation(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn build(&self, spec: &RunSpec) -> Result<Box<dyn SamplerAggregatorEngine>, BuildError>;
}

/// Loads immutable run configuration.
pub trait RunSpecStore: Send + Sync {
    async fn load_run_spec(&self, run_id: i32) -> Result<Option<RunSpec>, StoreError>;
}

/// Registers and monitors running component instances.
pub trait ComponentRegistryStore: Send + Sync {
    async fn register_instance(&self, instance: &ComponentInstance) -> Result<(), StoreError>;
    async fn heartbeat_instance(&self, instance_id: &str) -> Result<(), StoreError>;
    async fn update_instance_status(
        &self,
        instance_id: &str,
        status: InstanceStatus,
    ) -> Result<(), StoreError>;
    async fn get_instance(
        &self,
        instance_id: &str,
    ) -> Result<Option<ComponentInstance>, StoreError>;
}

/// Handles run assignment and lease ownership.
pub trait AssignmentLeaseStore: Send + Sync {
    async fn acquire_sampler_aggregator_lease(
        &self,
        run_id: i32,
        instance_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError>;
    async fn renew_sampler_aggregator_lease(
        &self,
        run_id: i32,
        instance_id: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError>;
    async fn release_sampler_aggregator_lease(
        &self,
        run_id: i32,
        instance_id: &str,
    ) -> Result<(), StoreError>;
    async fn assign_evaluator(&self, run_id: i32, instance_id: &str) -> Result<(), StoreError>;
    async fn unassign_evaluator(&self, run_id: i32, instance_id: &str) -> Result<(), StoreError>;
    async fn list_assigned_evaluators(&self, run_id: i32) -> Result<Vec<String>, StoreError>;
}

/// Accesses the batch work queue.
pub trait WorkQueueStore: Send + Sync {
    async fn insert_batch(&self, run_id: i32, batch: &Batch) -> Result<(), StoreError>;
    async fn get_pending_batch_count(&self, run_id: i32) -> Result<i64, StoreError>;
    async fn claim_batch(
        &self,
        run_id: i32,
        instance_id: &str,
    ) -> Result<Option<BatchClaim>, StoreError>;
    async fn submit_batch_results(
        &self,
        batch_id: i64,
        results: &BatchResults,
        eval_time_ms: f64,
    ) -> Result<(), StoreError>;
    async fn fail_batch(&self, batch_id: i64, last_error: &str) -> Result<(), StoreError>;
    async fn fetch_completed_batches_since(
        &self,
        run_id: i32,
        last_batch_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<CompletedBatch>, StoreError>;
}

/// Optional persistence for sampler-aggregator state snapshots.
pub trait EngineStateStore: Send + Sync {
    async fn load_engine_state(&self, run_id: i32) -> Result<Option<EngineState>, StoreError>;
    async fn save_engine_state(&self, run_id: i32, state: &EngineState) -> Result<(), StoreError>;
}

/// Persists aggregated run snapshots used by API/dashboard history.
pub trait AggregationStore: Send + Sync {
    async fn aggregate_and_persist(
        &self,
        run_id: i32,
        completed: &[CompletedBatch],
    ) -> Result<(), StoreError>;
}

/// Read-side store for API/dashboard endpoints.
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
}
