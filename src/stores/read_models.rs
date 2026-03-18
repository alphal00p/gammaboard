//! Read models for API/dashboard responses.

use crate::core::{EvaluatorPerformanceMetrics, SamplerPerformanceMetrics};
use crate::evaluation::PointSpec;
use serde::{Deserialize, Serialize};

/// Run progress information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProgress {
    pub run_id: i32,
    pub run_name: String,
    pub lifecycle_state: String,
    pub desired_assignment_count: i64,
    pub active_worker_count: i64,
    pub integration_params: Option<serde_json::Value>,
    pub point_spec: Option<PointSpec>,
    pub active_task_id: Option<String>,
    pub target: Option<serde_json::Value>,
    pub evaluator_init_metadata: Option<serde_json::Value>,
    pub sampler_aggregator_init_metadata: Option<serde_json::Value>,
    pub nr_produced_samples: i64,
    pub nr_completed_samples: i64,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub training_completed_at: Option<chrono::DateTime<chrono::Utc>>,
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

/// Task-local persisted observable snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutputSnapshot {
    pub id: String,
    pub run_id: i32,
    pub task_id: String,
    pub persisted_output: serde_json::Value,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStageSnapshot {
    pub id: String,
    pub run_id: i32,
    pub task_id: String,
    pub observable_state: ObservableState,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
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

/// Cursor-paged runtime log response for dashboard browsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerLogPage {
    pub items: Vec<WorkerLogEntry>,
    pub next_before_id: Option<String>,
    pub has_more_older: bool,
}

/// Registered node snapshot exposed through the `/nodes` read endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredWorkerEntry {
    pub worker_id: String,
    pub node_id: Option<String>,
    pub desired_run_id: Option<i32>,
    pub desired_role: Option<String>,
    pub current_run_id: Option<i32>,
    pub current_role: Option<String>,
    pub role: String,
    pub implementation: String,
    pub version: String,
    pub status: String,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub evaluator_metrics: Option<EvaluatorPerformanceMetrics>,
    pub sampler_metrics: Option<SamplerPerformanceMetrics>,
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
use crate::evaluation::ObservableState;
