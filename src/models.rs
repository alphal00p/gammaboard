//! API models for database responses.

use serde::{Deserialize, Serialize};

/// Run progress information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProgress {
    pub run_id: i32,
    pub run_status: String,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_batches_planned: Option<i32>,
    pub batches_completed: i32,
    pub final_result: Option<f64>,
    pub error_estimate: Option<f64>,
    pub total_batches: i64,
    pub total_samples: i64,
    pub pending_batches: i64,
    pub claimed_batches: i64,
    pub completed_batches: i64,
    pub failed_batches: i64,
    pub completion_rate: f64,
}

/// Work queue statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkQueueStats {
    pub run_id: i32,
    pub status: String,
    pub batch_count: i64,
    pub total_samples: i64,
    pub avg_batch_time_ms: Option<f64>,
    pub avg_sample_time_ms: Option<f64>,
}

/// Aggregated results snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregatedResult {
    pub id: i64,
    pub run_id: i32,
    pub nr_samples: i64,
    pub nr_batches: i32,
    pub sum: f64,
    pub sum_x2: f64,
    pub sum_abs: f64,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub weighted_sum: f64,
    pub weighted_sum_x2: f64,
    pub sum_weights: f64,
    pub effective_sample_size: Option<f64>,
    pub mean: Option<f64>,
    pub variance: Option<f64>,
    pub std_dev: Option<f64>,
    pub error_estimate: Option<f64>,
    pub histograms: Option<serde_json::Value>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}
