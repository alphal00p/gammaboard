use crate::core::BatchTransformConfig;
use crate::core::SamplerAggregatorConfig;
use crate::evaluation::{Batch, BatchResult, ObservableState};
use crate::sampling::{LatentBatch, SamplerAggregatorSnapshot};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRole {
    Evaluator,
    SamplerAggregator,
}

impl WorkerRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Evaluator => "evaluator",
            Self::SamplerAggregator => "sampler_aggregator",
        }
    }
}

impl fmt::Display for WorkerRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkerRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "evaluator" => Ok(Self::Evaluator),
            "sampler_aggregator" | "sampler-aggregator" => Ok(Self::SamplerAggregator),
            other => Err(format!("unknown worker role: {other}")),
        }
    }
}

/// Desired node-level role assignment managed by the control plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesiredAssignment {
    pub node_name: String,
    pub role: WorkerRole,
    pub run_id: i32,
    pub run_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredNode {
    pub name: String,
    pub uuid: String,
    pub desired_assignment: Option<DesiredAssignment>,
    pub current_assignment: Option<DesiredAssignment>,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct BatchClaim {
    pub batch_id: i64,
    pub task_id: i64,
    pub requires_training_values: bool,
    pub latent_batch: LatentBatch,
}

#[derive(Debug, Clone)]
pub struct CompletedBatch {
    pub batch_id: i64,
    pub task_id: i64,
    pub requires_training_values: bool,
    pub latent_batch: LatentBatch,
    pub result: BatchResult,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_eval_time_ms: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct BatchQueueCounts {
    pub pending: i64,
    pub claimed: i64,
    pub completed: i64,
}

impl BatchQueueCounts {
    pub fn runnable(self) -> i64 {
        self.pending + self.claimed
    }

    pub fn open(self) -> i64 {
        self.pending + self.claimed + self.completed
    }
}

/// Status of a batch in the work queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
}

impl BatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BatchStatus::Pending => "pending",
            BatchStatus::Claimed => "claimed",
            BatchStatus::Completed => "completed",
            BatchStatus::Failed => "failed",
        }
    }
}

/// A concrete batch with metadata from the database.
#[derive(Debug, Clone)]
pub struct BatchRecord {
    pub id: i64,
    pub run_id: i32,
    pub batch: Batch,
    pub status: BatchStatus,
    pub claimed_by_node_name: Option<String>,
    pub claimed_by_node_uuid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLogEvent {
    pub source: String,
    pub run_id: Option<i32>,
    pub node_uuid: Option<String>,
    pub node_name: Option<String>,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorIdleProfileMetrics {
    pub idle_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EvaluatorPerformanceMetrics {
    pub batches_completed: i64,
    pub samples_evaluated: i64,
    pub avg_time_per_sample_ms: f64,
    pub std_time_per_sample_ms: f64,
    pub avg_evaluate_time_per_sample_ms: f64,
    pub std_evaluate_time_per_sample_ms: f64,
    pub avg_materialization_time_per_sample_ms: f64,
    pub std_materialization_time_per_sample_ms: f64,
    pub idle_profile: Option<EvaluatorIdleProfileMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerPerformanceMetrics {
    pub produced_batches: i64,
    pub produced_samples: i64,
    pub avg_produce_time_per_sample_ms: f64,
    pub std_produce_time_per_sample_ms: f64,
    pub ingested_batches: i64,
    pub ingested_samples: i64,
    pub avg_ingest_time_per_sample_ms: f64,
    pub std_ingest_time_per_sample_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollingMetricSnapshot {
    pub mean: Option<f64>,
    pub std_dev: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerRollingAverages {
    pub eval_ms_per_sample: RollingMetricSnapshot,
    pub eval_ms_per_batch: RollingMetricSnapshot,
    pub sampler_produce_ms_per_sample: RollingMetricSnapshot,
    pub sampler_ingest_ms_per_sample: RollingMetricSnapshot,
    pub queue_remaining_ratio: RollingMetricSnapshot,
    pub batches_consumed_per_tick: RollingMetricSnapshot,
    pub batches_consumed_per_second: RollingMetricSnapshot,
    pub sampler_tick_ms: RollingMetricSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerRuntimeMetrics {
    pub produced_batches_total: i64,
    pub produced_samples_total: i64,
    pub ingested_batches_total: i64,
    pub ingested_samples_total: i64,
    pub completed_samples_per_second: f64,
    pub batch_size_current: usize,
    pub rolling: SamplerRollingAverages,
}

impl SamplerRuntimeMetrics {
    pub fn to_performance_metrics(&self) -> SamplerPerformanceMetrics {
        SamplerPerformanceMetrics {
            produced_batches: self.produced_batches_total,
            produced_samples: self.produced_samples_total,
            avg_produce_time_per_sample_ms: self
                .rolling
                .sampler_produce_ms_per_sample
                .mean
                .unwrap_or(0.0),
            std_produce_time_per_sample_ms: self.rolling.sampler_produce_ms_per_sample.std_dev,
            ingested_batches: self.ingested_batches_total,
            ingested_samples: self.ingested_samples_total,
            avg_ingest_time_per_sample_ms: self
                .rolling
                .sampler_ingest_ms_per_sample
                .mean
                .unwrap_or(0.0),
            std_ingest_time_per_sample_ms: self.rolling.sampler_ingest_ms_per_sample.std_dev,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorPerformanceSnapshot {
    pub run_id: i32,
    pub node_name: String,
    pub metrics: EvaluatorPerformanceMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerAggregatorPerformanceSnapshot {
    pub run_id: i32,
    pub node_name: String,
    pub runtime_metrics: SamplerRuntimeMetrics,
    pub engine_diagnostics: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSampleProgress {
    pub nr_produced_samples: i64,
    pub nr_completed_samples: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStageSnapshot {
    pub id: Option<i64>,
    pub run_id: i32,
    pub task_id: Option<i64>,
    pub name: String,
    pub sequence_nr: Option<i32>,
    pub queue_empty: bool,
    pub sampler_snapshot: Option<SamplerAggregatorSnapshot>,
    pub observable_state: Option<ObservableState>,
    pub sampler_aggregator: Option<SamplerAggregatorConfig>,
    pub batch_transforms: Vec<BatchTransformConfig>,
}
