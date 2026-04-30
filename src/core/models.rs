use crate::core::BatchTransformConfig;
use crate::core::SamplerAggregatorConfig;
use crate::evaluation::{AccumulatorState, Batch, BatchResult};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeLaunchRequest {
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_i64_as_string")]
    pub id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub state: String,
    pub backend: String,
    pub requested_count: i32,
    pub started_count: i32,
    pub name_prefix: Option<String>,
    pub args: JsonValue,
    pub result: JsonValue,
    pub error: Option<String>,
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
    pub batch_size: usize,
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
    pub avg_fetch_time_per_sample_ms: f64,
    pub std_fetch_time_per_sample_ms: f64,
    pub avg_fetch_stall_time_per_sample_ms: f64,
    pub std_fetch_stall_time_per_sample_ms: f64,
    pub prefetch_hit_ratio: f64,
    pub fetch_stall_ratio: f64,
    pub queue_starvation_ratio: f64,
    pub avg_evaluate_time_per_sample_ms: f64,
    pub std_evaluate_time_per_sample_ms: f64,
    pub avg_materialization_time_per_sample_ms: f64,
    pub std_materialization_time_per_sample_ms: f64,
    pub avg_submit_time_per_sample_ms: f64,
    pub std_submit_time_per_sample_ms: f64,
    pub avg_submit_stall_time_per_sample_ms: f64,
    pub std_submit_stall_time_per_sample_ms: f64,
    pub submit_slot_hit_ratio: f64,
    pub submit_stall_ratio: f64,
    #[serde(default)]
    pub completed_samples_total: i64,
    pub idle_profile: Option<EvaluatorIdleProfileMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SamplerPerformanceMetrics {
    pub produced_batches: i64,
    pub produced_samples: i64,
    pub avg_produce_time_per_sample_ms: f64,
    pub std_produce_time_per_sample_ms: f64,
    pub ingested_batches: i64,
    pub ingested_samples: i64,
    pub avg_ingest_time_per_sample_ms: f64,
    pub std_ingest_time_per_sample_ms: f64,
    #[serde(default)]
    pub completed_samples_total: i64,
    #[serde(default)]
    pub sampler_uptime_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RollingMetricSnapshot {
    #[serde(default)]
    pub count: u64,
    pub mean: Option<f64>,
    #[serde(default)]
    pub total: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    pub std_dev: f64,
}

#[derive(Debug, Clone, Default)]
pub struct InsertBatchesMetrics {
    pub serialize_ms: f64,
    pub payload_bytes: usize,
    pub insert_batches_exec_ms: f64,
    pub insert_inputs_exec_ms: f64,
    pub commit_ms: f64,
    pub end_to_end_ms: f64,
}

#[derive(Debug, Clone, Default)]
pub struct InsertBatchesOutcome {
    pub batch_ids: Vec<i64>,
    pub metrics: InsertBatchesMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SamplerWorkRollingAverages {
    pub eval_ms_per_sample: RollingMetricSnapshot,
    pub eval_ms_per_batch: RollingMetricSnapshot,
    pub training_ingest_ms_per_sample: RollingMetricSnapshot,
    pub completed_training_ingest_ms: RollingMetricSnapshot,
    pub produce_ms_per_sample: RollingMetricSnapshot,
    pub reclaim_ms: RollingMetricSnapshot,
    pub queue_counts_ms: RollingMetricSnapshot,
    pub completed_merge_ingest_ms: RollingMetricSnapshot,
    pub persist_accumulator_ms: RollingMetricSnapshot,
    pub completed_delete_ms: RollingMetricSnapshot,
    pub produce_ms: RollingMetricSnapshot,
    pub progress_sync_ms: RollingMetricSnapshot,
    pub performance_sync_ms: RollingMetricSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SamplerQueueRollingAverages {
    pub fetch_completed_ms: RollingMetricSnapshot,
    pub fetch_completed_batches: RollingMetricSnapshot,
    pub fetch_completed_prefetch_fill_ratio: RollingMetricSnapshot,
    pub insert_bundle_ms: RollingMetricSnapshot,
    pub insert_bundle_batches: RollingMetricSnapshot,
    pub insert_bundle_ms_per_batch: RollingMetricSnapshot,
    pub insert_bundle_serialize_ms: RollingMetricSnapshot,
    pub insert_bundle_payload_bytes: RollingMetricSnapshot,
    pub insert_bundle_payload_bytes_per_batch: RollingMetricSnapshot,
    pub insert_bundle_db_batches_ms: RollingMetricSnapshot,
    pub insert_bundle_db_inputs_ms: RollingMetricSnapshot,
    pub insert_bundle_commit_ms: RollingMetricSnapshot,
    pub insert_bundle_local_pending_at_start: RollingMetricSnapshot,
    pub insert_bundle_db_pending_at_start: RollingMetricSnapshot,
}

impl Default for SamplerQueueRollingAverages {
    fn default() -> Self {
        Self {
            fetch_completed_ms: RollingMetricSnapshot::default(),
            fetch_completed_batches: RollingMetricSnapshot::default(),
            fetch_completed_prefetch_fill_ratio: RollingMetricSnapshot::default(),
            insert_bundle_ms: RollingMetricSnapshot::default(),
            insert_bundle_batches: RollingMetricSnapshot::default(),
            insert_bundle_ms_per_batch: RollingMetricSnapshot::default(),
            insert_bundle_serialize_ms: RollingMetricSnapshot::default(),
            insert_bundle_payload_bytes: RollingMetricSnapshot::default(),
            insert_bundle_payload_bytes_per_batch: RollingMetricSnapshot::default(),
            insert_bundle_db_batches_ms: RollingMetricSnapshot::default(),
            insert_bundle_db_inputs_ms: RollingMetricSnapshot::default(),
            insert_bundle_commit_ms: RollingMetricSnapshot::default(),
            insert_bundle_local_pending_at_start: RollingMetricSnapshot::default(),
            insert_bundle_db_pending_at_start: RollingMetricSnapshot::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SamplerQueueRuntimeMetrics {
    pub db_pending_batches: Option<i64>,
    pub db_claimed_batches: Option<i64>,
    pub db_completed_batches: Option<i64>,
    pub local_pending_batches: usize,
    pub local_inflight_insert_tasks: usize,
    pub local_inflight_insert_batches: usize,
    pub local_ready_processed_batches: usize,
    pub insert_task_utilization: Option<f64>,
    pub completed_fetch_utilization: Option<f64>,
    pub rolling: SamplerQueueRollingAverages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerRuntimeMetrics {
    pub produced_batches_total: i64,
    pub produced_samples_total: i64,
    pub ingested_batches_total: i64,
    pub ingested_samples_total: i64,
    #[serde(default)]
    pub completed_samples_total: i64,
    #[serde(default)]
    pub sampler_uptime_ms: f64,
    pub completed_samples_per_second: f64,
    #[serde(default)]
    pub eta_completed_samples_per_second: f64,
    #[serde(default)]
    pub eta_seconds_smoothed: Option<f64>,
    pub batch_size_current: usize,
    pub sampler_tick_busy_ratio: Option<f64>,
    #[serde(default)]
    pub avg_evaluator_utilization: Option<f64>,
    #[serde(default)]
    pub active_evaluator_count: Option<usize>,
    #[serde(default)]
    pub avg_evaluator_rss_bytes: Option<i64>,
    #[serde(default)]
    pub total_evaluator_rss_bytes: Option<i64>,
    pub sampler: SamplerWorkRollingAverages,
    pub queue: SamplerQueueRuntimeMetrics,
}

impl SamplerRuntimeMetrics {
    pub fn to_performance_metrics(&self) -> SamplerPerformanceMetrics {
        SamplerPerformanceMetrics {
            produced_batches: self.produced_batches_total,
            produced_samples: self.produced_samples_total,
            avg_produce_time_per_sample_ms: self.sampler.produce_ms_per_sample.mean.unwrap_or(0.0),
            std_produce_time_per_sample_ms: self.sampler.produce_ms_per_sample.std_dev,
            ingested_batches: self.ingested_batches_total,
            ingested_samples: self.ingested_samples_total,
            avg_ingest_time_per_sample_ms: self
                .sampler
                .training_ingest_ms_per_sample
                .mean
                .unwrap_or(0.0),
            std_ingest_time_per_sample_ms: self.sampler.training_ingest_ms_per_sample.std_dev,
            completed_samples_total: self.completed_samples_total,
            sampler_uptime_ms: self.sampler_uptime_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorPerformanceSnapshot {
    pub run_id: i32,
    pub node_name: String,
    pub metrics: EvaluatorPerformanceMetrics,
    pub rss_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerAggregatorPerformanceSnapshot {
    pub run_id: i32,
    pub node_name: String,
    pub runtime_metrics: SamplerRuntimeMetrics,
    pub engine_diagnostics: JsonValue,
    pub rss_bytes: Option<i64>,
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
    pub observable_state: Option<AccumulatorState>,
    pub sampler_aggregator: Option<SamplerAggregatorConfig>,
    pub batch_transforms: Vec<BatchTransformConfig>,
}
