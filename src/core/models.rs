use crate::core::BatchTransformConfig;
use crate::core::{AccumulatorMetricName, EvaluatorConfig, SamplingConfig};
use crate::evaluation::{AccumulatorState, BatchResult};
use crate::sampling::{LatentBatch, SavedSampler};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{collections::BTreeMap, fmt, str::FromStr};

pub type NodeCapabilities = BTreeMap<String, u64>;

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
            "sampler_aggregator" => Ok(Self::SamplerAggregator),
            other => Err(format!("unknown worker role: {other}")),
        }
    }
}

/// A placement or pool target; node identity belongs to the enclosing worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignmentTarget {
    pub role: WorkerRole,
    pub run_id: i32,
}

/// One conditional assignment change. A batch is published atomically only if
/// every node still has the UUID and desired assignment observed by the planner.
#[derive(Debug, Clone)]
pub struct NodeAssignmentUpdate {
    pub node_uuid: String,
    pub expected: Option<AssignmentTarget>,
    pub desired: Option<AssignmentTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredNode {
    pub name: String,
    pub uuid: String,
    pub capabilities: NodeCapabilities,
    /// Operator-owned pool, independent of scheduled placement.
    #[serde(default)]
    pub pool_assignment: Option<AssignmentTarget>,
    pub desired_assignment: Option<AssignmentTarget>,
    pub current_assignment: Option<AssignmentTarget>,
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

/// Runtime-owned work: a sample slice plus its resolved accumulator configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationBatch {
    pub stratification: Option<super::StratificationPlan>,
    pub samples: LatentBatch,
    pub accumulator: super::AccumulatorConfig,
}

impl EvaluationBatch {
    pub fn to_bytes(&self) -> Result<Vec<u8>, crate::evaluation::BatchError> {
        let mut bytes = Vec::new();
        self.write_bytes(&mut bytes)?;
        Ok(bytes)
    }

    pub(crate) fn write_bytes(
        &self,
        bytes: &mut Vec<u8>,
    ) -> Result<usize, crate::evaluation::BatchError> {
        let start = bytes.len();
        bytes.extend_from_slice(crate::compatibility::QUEUE_MAGIC);
        bincode::serde::encode_into_std_write(
            &self.accumulator,
            bytes,
            bincode::config::standard(),
        )
        .map_err(|e| crate::evaluation::BatchError::layout(e.to_string()))?;
        bincode::serde::encode_into_std_write(
            serde_json::to_string(&self.stratification)
                .map_err(|e| crate::evaluation::BatchError::layout(e.to_string()))?,
            bytes,
            bincode::config::standard(),
        )
        .map_err(|e| crate::evaluation::BatchError::layout(e.to_string()))?;
        self.samples.write_bytes(bytes)?;
        Ok(bytes.len() - start)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, crate::evaluation::BatchError> {
        let bytes = bytes
            .strip_prefix(crate::compatibility::QUEUE_MAGIC)
            .ok_or_else(|| {
                crate::evaluation::BatchError::layout(
                    "unsupported evaluation queue format; recreate the run with matching workers",
                )
            })?;
        let (accumulator, consumed) =
            bincode::serde::decode_from_slice(bytes, bincode::config::standard())
                .map_err(|e| crate::evaluation::BatchError::layout(e.to_string()))?;
        let (plan, plan_bytes): (String, usize) =
            bincode::serde::decode_from_slice(&bytes[consumed..], bincode::config::standard())
                .map_err(|e| crate::evaluation::BatchError::layout(e.to_string()))?;
        let stratification = serde_json::from_str(&plan)
            .map_err(|e| crate::evaluation::BatchError::layout(e.to_string()))?;
        Ok(Self {
            stratification,
            accumulator,
            samples: LatentBatch::from_bytes(&bytes[consumed + plan_bytes..])?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BatchClaim {
    pub stratification: Option<super::StratificationPlan>,
    pub batch_id: i64,
    pub claim_token: String,
    pub task_id: i64,
    pub requires_training_values: bool,
    pub latent_batch: LatentBatch,
    pub accumulator: super::AccumulatorConfig,
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
    pub failed: i64,
}

impl BatchQueueCounts {
    pub fn open(self) -> i64 {
        self.pending + self.claimed + self.completed
    }
}

/// The first unfinished batch after the sampler's task-local completion cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueBlocker {
    #[serde(
        serialize_with = "crate::utils::serde_bigint::serialize_i64_as_string",
        deserialize_with = "crate::utils::serde_bigint::deserialize_i64_from_string_or_number"
    )]
    pub batch_id: i64,
    pub status: String,
    pub node_name: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BasicEstimate {
    pub mean: f64,
    pub error: Option<f64>,
}

/// Identity of the estimator whose uncertainty drives measurements and control.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EstimateKind {
    #[default]
    Pooled,
    Stratified,
    Combined,
}

impl EstimateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pooled => "pooled",
            Self::Stratified => "stratified",
            Self::Combined => "combined",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeasurementResult {
    pub estimator: EstimateKind,
    pub basic: Option<BasicEstimate>,
    pub name: AccumulatorMetricName,
    pub component: Option<String>,
    pub value: f64,
    pub uncertainty: Option<f64>,
    pub sample_count: i64,
}

/// Identifies the exact child result used by a controller. Active results do
/// not have a stage snapshot yet, so their sample count is the live revision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResultSourceRef {
    pub run_id: i32,
    #[serde(
        serialize_with = "crate::utils::serde_bigint::serialize_i64_as_string",
        deserialize_with = "crate::utils::serde_bigint::deserialize_i64_from_string_or_number"
    )]
    pub task_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    pub sample_count: i64,
}

/// Scientific result materialized above accumulator state. Metrics remain
/// small selector projections; observables contain normalized, directly
/// composable values and uncertainties.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DerivedResultSnapshot {
    pub sources: Vec<ResultSourceRef>,
    pub metrics: Vec<MeasurementResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observables: Option<serde_json::Value>,
}

/// Occupied wall seconds since runner initialization, using one monotonic clock.
/// Compute and I/O overlap; concurrent I/O counts once.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkerBusyMetrics {
    pub elapsed_seconds: f64,
    pub compute_seconds: f64,
    pub io_seconds: f64,
}

/// Identity and timing shared by all worker reports. A task may be absent while idle.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkerReportContext {
    pub epoch: String,
    pub node_uuid: String,
    pub task_id: Option<String>,
    pub busy: WorkerBusyMetrics,
}

/// Cumulative wall seconds in one runner epoch. Operation costs cover
/// successfully submitted batches; busy time also includes failed attempts.
/// Asynchronous phases overlap; these are not an additive wall-time breakdown.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EvaluatorCumulativeMetrics {
    pub evaluate_seconds: f64,
    pub materialize_seconds: f64,
    pub fetch_wait_seconds: f64,
    pub submit_seconds: f64,
    pub submit_wait_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvaluatorPerformanceMetrics {
    #[serde(flatten)]
    pub worker: WorkerReportContext,
    pub cumulative: EvaluatorCumulativeMetrics,
    pub engine_diagnostics: serde_json::Value,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RollingMetricSnapshot {
    #[serde(default)]
    pub count: u64,
    pub mean: Option<f64>,
    pub total: Option<f64>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchFailOutcome {
    Requeued { task_id: i64, retry_count: i32 },
    PermanentlyFailed { task_id: i64, retry_count: i32 },
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SamplerQueueRuntimeMetrics {
    pub blocker: Option<QueueBlocker>,
    pub db_pending_batches: Option<i64>,
    pub db_claimed_batches: Option<i64>,
    pub db_completed_batches: Option<i64>,
    pub local_pending_batches: usize,
    pub local_inflight_insert_tasks: usize,
    pub local_inflight_insert_batches: usize,
    pub local_ready_processed_batches: usize,
    pub rolling: SamplerQueueRollingAverages,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerPerformanceMetrics {
    #[serde(flatten)]
    pub worker: WorkerReportContext,
    pub engine_diagnostics: JsonValue,
    pub queue_config: crate::runners::SamplerQueueConfig,
    pub target_pending_batches: Option<usize>,
    pub buffered_generated_samples: usize,
    pub(crate) checkpoint_state: super::checkpoint::AccumulatorCheckpointState,
    pub produced_batches_total: i64,
    pub produced_samples_total: i64,
    pub ingested_batches_total: i64,
    pub ingested_samples_total: i64,
    #[serde(default)]
    pub completed_samples_total: i64,
    #[serde(default)]
    pub sampler_uptime_ms: f64,
    pub completed_samples_per_second: f64,
    pub eta_seconds: Option<f64>,
    pub batch_size_current: usize,
    pub active_evaluator_count: Option<usize>,
    pub avg_evaluator_rss_bytes: Option<i64>,
    pub total_evaluator_rss_bytes: Option<i64>,
    pub sampler: SamplerWorkRollingAverages,
    pub queue: SamplerQueueRuntimeMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerPerformanceSnapshot<M> {
    pub run_id: i32,
    pub node_name: String,
    pub metrics: M,
    pub rss_bytes: Option<i64>,
}

pub type EvaluatorPerformanceSnapshot = WorkerPerformanceSnapshot<EvaluatorPerformanceMetrics>;
pub type SamplerAggregatorPerformanceSnapshot =
    WorkerPerformanceSnapshot<SamplerPerformanceMetrics>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunSampleProgress {
    #[serde(default)]
    pub active_task_id: Option<i64>,
    pub nr_produced_samples: i64,
    pub nr_completed_samples: i64,
    pub sampler_runner_uptime_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStageSnapshot {
    pub id: Option<i64>,
    pub run_id: i32,
    pub task_id: Option<i64>,
    pub name: String,
    pub sequence_nr: Option<i32>,
    pub queue_empty: bool,
    pub sampler_snapshot: Option<SavedSampler>,
    pub observable_state: Option<AccumulatorState>,
    pub evaluator: Option<EvaluatorConfig>,
    pub batch_transforms: Vec<BatchTransformConfig>,
}

impl RunStageSnapshot {
    pub fn sampler_config(&self) -> Option<&SamplingConfig> {
        self.sampler_snapshot.as_ref().map(|saved| &saved.config)
    }
}

#[cfg(test)]
mod tests {
    use super::{ResultSourceRef, WorkerRole};
    use std::str::FromStr;

    #[test]
    fn worker_reports_require_identity_and_clock() {
        let report = serde_json::to_value(super::WorkerReportContext::default()).unwrap();
        for field in ["epoch", "node_uuid", "busy"] {
            let mut incomplete = report.clone();
            incomplete.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<super::WorkerReportContext>(incomplete).is_err(),
                "report silently accepted missing {field}"
            );
        }
    }

    #[test]
    fn queue_blocker_survives_persisted_performance_roundtrip() {
        let metrics = super::SamplerQueueRuntimeMetrics {
            blocker: Some(super::QueueBlocker {
                batch_id: i64::MAX,
                status: "claimed".into(),
                node_name: Some("worker".into()),
                claimed_at: Some(chrono::Utc::now()),
            }),
            ..Default::default()
        };
        let json = serde_json::to_value(&metrics).unwrap();
        assert_eq!(json["blocker"]["batch_id"], i64::MAX.to_string());
        let restored: super::SamplerQueueRuntimeMetrics = serde_json::from_value(json).unwrap();
        assert_eq!(restored.blocker.unwrap().batch_id, i64::MAX);
        assert!(
            serde_json::from_str::<super::SamplerQueueRuntimeMetrics>("{}")
                .unwrap()
                .blocker
                .is_none()
        );
    }

    #[test]
    fn queued_samples_roundtrip_with_their_accumulator() {
        let samples =
            crate::sampling::LatentBatch::from_batch(&crate::evaluation::Batch::from_iter([
                crate::evaluation::Point::new(vec![0.25], vec![], 2.0),
            ]));
        let work = samples.for_evaluation(crate::core::AccumulatorConfig::scalar());
        let bytes = work.to_bytes().unwrap();
        assert_eq!(super::EvaluationBatch::from_bytes(&bytes).unwrap(), work);
        let mut trailing = bytes;
        trailing.push(0);
        assert!(super::EvaluationBatch::from_bytes(&trailing).is_err());
    }

    #[test]
    fn worker_role_accepts_only_canonical_spelling() {
        assert_eq!(
            WorkerRole::from_str("sampler_aggregator"),
            Ok(WorkerRole::SamplerAggregator)
        );
        assert!(WorkerRole::from_str("sampler-aggregator").is_err());
    }

    #[test]
    fn result_source_reference_roundtrips_json_safe_ids() {
        let source = ResultSourceRef {
            run_id: 3,
            task_id: i64::MAX,
            snapshot_id: Some("7".to_string()),
            sample_count: 42,
        };
        let value = serde_json::to_value(&source).expect("serialize");
        assert_eq!(value["task_id"], i64::MAX.to_string());
        assert_eq!(
            serde_json::from_value::<ResultSourceRef>(value).expect("deserialize"),
            source
        );
    }
}
