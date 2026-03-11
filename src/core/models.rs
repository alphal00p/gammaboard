use crate::core::{Batch, BatchResult};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Active,
    Draining,
    Inactive,
}

impl WorkerStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Draining => "draining",
            Self::Inactive => "inactive",
        }
    }
}

impl fmt::Display for WorkerStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WorkerStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "draining" => Ok(Self::Draining),
            "inactive" => Ok(Self::Inactive),
            other => Err(format!("unknown worker status: {other}")),
        }
    }
}

/// Registered running process (role-specific identity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worker {
    pub worker_id: String,
    pub node_id: Option<String>,
    pub role: WorkerRole,
    pub implementation: String,
    pub version: String,
    pub node_specs: JsonValue,
    pub status: WorkerStatus,
    pub last_seen: Option<DateTime<Utc>>,
}

/// Assignment lease for a run/worker pairing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentLease {
    pub run_id: i32,
    pub role: WorkerRole,
    pub worker_id: String,
    pub lease_expires_at: DateTime<Utc>,
}

/// Desired node-level role assignment managed by the control plane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesiredAssignment {
    pub node_id: String,
    pub role: WorkerRole,
    pub run_id: i32,
}

#[derive(Debug, Clone)]
pub struct BatchClaim {
    pub batch_id: i64,
    pub batch: Batch,
    pub requires_training: bool,
}

#[derive(Debug, Clone)]
pub struct CompletedBatch {
    pub batch_id: i64,
    pub batch: Batch,
    pub requires_training: bool,
    pub result: BatchResult,
    pub completed_at: Option<DateTime<Utc>>,
    pub total_eval_time_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLogEvent {
    pub source: String,
    pub run_id: Option<i32>,
    pub node_id: Option<String>,
    pub worker_id: Option<String>,
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorIdleProfileMetrics {
    pub idle_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorPerformanceMetrics {
    pub batches_completed: i64,
    pub samples_evaluated: i64,
    pub avg_time_per_sample_ms: f64,
    pub std_time_per_sample_ms: f64,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerRuntimeMetrics {
    pub produced_batches_total: i64,
    pub produced_samples_total: i64,
    pub ingested_batches_total: i64,
    pub ingested_samples_total: i64,
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
    pub worker_id: String,
    pub metrics: EvaluatorPerformanceMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerAggregatorPerformanceSnapshot {
    pub run_id: i32,
    pub worker_id: String,
    pub runtime_metrics: SamplerRuntimeMetrics,
    pub engine_diagnostics: JsonValue,
}
