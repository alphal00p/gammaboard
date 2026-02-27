//! Read models for API/dashboard responses.

use crate::core::RunStatus;
use serde::{Deserialize, Serialize};

/// Run progress information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProgress {
    pub run_id: i32,
    pub run_name: String,
    pub run_status: RunStatus,
    pub integration_params: Option<serde_json::Value>,
    pub evaluator_init_metadata: Option<serde_json::Value>,
    pub sampler_aggregator_init_metadata: Option<serde_json::Value>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
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
    pub id: i64,
    pub run_id: i32,
    pub aggregated_observable: serde_json::Value,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Worker log event persisted from `run_node` tracing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerLogEntry {
    pub id: i64,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub run_id: Option<i32>,
    pub node_id: Option<String>,
    pub worker_id: String,
    pub role: String,
    pub level: String,
    pub event_type: String,
    pub message: String,
    pub fields: serde_json::Value,
}

/// Registered worker process snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredWorkerEntry {
    pub worker_id: String,
    pub node_id: Option<String>,
    pub role: String,
    pub implementation: String,
    pub version: String,
    pub status: String,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub batches_completed: Option<i64>,
    pub samples_evaluated: Option<i64>,
    pub avg_time_per_sample_ms: Option<f64>,
    pub std_time_per_sample_ms: Option<f64>,
    pub produced_batches: Option<i64>,
    pub produced_samples: Option<i64>,
    pub avg_produce_time_per_sample_ms: Option<f64>,
    pub std_produce_time_per_sample_ms: Option<f64>,
    pub ingested_batches: Option<i64>,
    pub ingested_samples: Option<i64>,
    pub avg_ingest_time_per_sample_ms: Option<f64>,
    pub std_ingest_time_per_sample_ms: Option<f64>,
    pub evaluator_diagnostics: Option<serde_json::Value>,
    pub sampler_diagnostics: Option<serde_json::Value>,
}

/// Evaluator performance history row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorPerformanceHistoryEntry {
    pub id: i64,
    pub run_id: i32,
    pub worker_id: String,
    pub window_start: chrono::DateTime<chrono::Utc>,
    pub window_end: chrono::DateTime<chrono::Utc>,
    pub batches_completed: i64,
    pub samples_evaluated: i64,
    pub avg_time_per_sample_ms: f64,
    pub std_time_per_sample_ms: f64,
    pub diagnostics: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Sampler-aggregator performance history row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerPerformanceHistoryEntry {
    pub id: i64,
    pub run_id: i32,
    pub worker_id: String,
    pub window_start: chrono::DateTime<chrono::Utc>,
    pub window_end: chrono::DateTime<chrono::Utc>,
    pub produced_batches: i64,
    pub produced_samples: i64,
    pub avg_produce_time_per_sample_ms: f64,
    pub std_produce_time_per_sample_ms: f64,
    pub ingested_batches: i64,
    pub ingested_samples: i64,
    pub avg_ingest_time_per_sample_ms: f64,
    pub std_ingest_time_per_sample_ms: f64,
    pub diagnostics: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
