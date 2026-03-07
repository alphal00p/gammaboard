//! Read models for API/dashboard responses.

use crate::core::{EvaluatorPerformanceMetrics, RunStatus, SamplerPerformanceMetrics};
use serde::{Deserialize, Serialize};

/// Run progress information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProgress {
    pub run_id: i32,
    pub run_name: String,
    pub run_status: RunStatus,
    pub integration_params: Option<serde_json::Value>,
    pub target: Option<serde_json::Value>,
    pub evaluator_init_metadata: Option<serde_json::Value>,
    pub sampler_aggregator_init_metadata: Option<serde_json::Value>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub training_completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_batches_planned: Option<i32>,
    pub batches_completed: i32,
    pub total_batches: i64,
    pub total_samples: i64,
    pub pending_batches: i64,
    pub claimed_batches: i64,
    pub completed_batches: i64,
    pub failed_batches: i64,
    pub completion_rate: f64,
}

/// Work queue statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkQueueStats {
    pub run_id: i32,
    pub status: String,
    pub batch_count: i64,
    pub total_samples: i64,
    pub avg_batch_time_ms: Option<f64>,
    pub avg_sample_time_ms: Option<f64>,
}

/// Aggregated observable snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedResult {
    pub id: String,
    pub run_id: i32,
    pub aggregated_observable: serde_json::Value,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Sampled aggregated-history range response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedRangeResponse {
    pub snapshots: Vec<AggregatedResult>,
    pub latest: Option<AggregatedResult>,
    pub meta: AggregatedRangeMeta,
    pub reset_required: bool,
}

/// Metadata for sampled range responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedRangeMeta {
    pub abs_start: Option<i64>,
    pub abs_stop: Option<i64>,
    pub step: i64,
    pub latest_id: Option<String>,
    pub max_points: i64,
}

/// Worker log event persisted from runtime tracing (`source='worker'`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerLogEntry {
    pub id: String,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub run_id: Option<i32>,
    pub node_id: Option<String>,
    pub worker_id: Option<String>,
    pub level: String,
    pub message: String,
    pub fields: serde_json::Value,
}

/// Registered worker process snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredWorkerEntry {
    pub worker_id: String,
    pub node_id: Option<String>,
    pub desired_run_id: Option<i32>,
    pub role: String,
    pub implementation: String,
    pub version: String,
    pub status: String,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub evaluator_metrics: Option<EvaluatorPerformanceMetrics>,
    pub sampler_metrics: Option<SamplerPerformanceMetrics>,
    pub evaluator_engine_diagnostics: Option<serde_json::Value>,
    pub sampler_runtime_metrics: Option<serde_json::Value>,
    pub sampler_engine_diagnostics: Option<serde_json::Value>,
}

/// Evaluator performance history row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorPerformanceHistoryEntry {
    pub id: i64,
    pub run_id: i32,
    pub worker_id: String,
    pub metrics: EvaluatorPerformanceMetrics,
    pub engine_diagnostics: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Sampler-aggregator performance history row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerPerformanceHistoryEntry {
    pub id: i64,
    pub run_id: i32,
    pub worker_id: String,
    pub metrics: SamplerPerformanceMetrics,
    pub runtime_metrics: serde_json::Value,
    pub engine_diagnostics: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Evaluator performance history for a worker resolved from current assignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEvaluatorPerformanceHistoryResponse {
    pub run_id: Option<i32>,
    pub entries: Vec<EvaluatorPerformanceHistoryEntry>,
}

/// Sampler-aggregator performance history for a worker resolved from current assignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSamplerPerformanceHistoryResponse {
    pub run_id: Option<i32>,
    pub entries: Vec<SamplerPerformanceHistoryEntry>,
}
