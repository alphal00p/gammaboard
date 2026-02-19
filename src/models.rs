//! API models for database responses.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    #[serde(rename = "warm-up")]
    WarmUp,
    Running,
    Completed,
    Paused,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Pending => "pending",
            RunStatus::WarmUp => "warm-up",
            RunStatus::Running => "running",
            RunStatus::Completed => "completed",
            RunStatus::Paused => "paused",
            RunStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(RunStatus::Pending),
            "warm-up" | "warm_up" => Some(RunStatus::WarmUp),
            "running" => Some(RunStatus::Running),
            "completed" => Some(RunStatus::Completed),
            "paused" => Some(RunStatus::Paused),
            "cancelled" => Some(RunStatus::Cancelled),
            _ => None,
        }
    }
}

/// Run progress information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunProgress {
    pub run_id: i32,
    pub run_status: RunStatus,
    pub integration_params: Option<serde_json::Value>,
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
    pub aggregated_observable: serde_json::Value,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}
