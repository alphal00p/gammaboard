use super::read_models::{
    AggregatedRangeResponse, AggregatedResult, EvaluatorPerformanceHistoryEntry,
    RegisteredWorkerEntry, RunProgress, SamplerPerformanceHistoryEntry, WorkQueueStats,
    WorkerEvaluatorPerformanceHistoryResponse, WorkerLogEntry,
    WorkerSamplerPerformanceHistoryResponse,
};
use crate::core::StoreError;
use async_trait::async_trait;

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
        after_id: Option<i64>,
    ) -> Result<Vec<WorkerLogEntry>, StoreError>;
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
