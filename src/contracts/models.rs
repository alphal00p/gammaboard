use crate::{Batch, BatchResults};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Immutable run configuration loaded from storage before starting a run loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: i32,
    pub worker_implementation: String,
    pub worker_version: String,
    pub worker_params: JsonValue,
    pub sampler_aggregator_implementation: String,
    pub sampler_aggregator_version: String,
    pub sampler_aggregator_params: JsonValue,
    pub worker_runner_params: JsonValue,
    pub sampler_aggregator_runner_params: JsonValue,
}

/// In-memory state passed to a sampler-aggregator during initialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineState {
    pub last_processed_batch_id: Option<i64>,
    pub state: JsonValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRole {
    Evaluator,
    SamplerAggregator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Active,
    Draining,
    Inactive,
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
}

#[derive(Debug, Clone)]
pub struct CompletedBatch {
    pub batch_id: i64,
    pub batch: Batch,
    pub results: BatchResults,
    pub completed_at: Option<DateTime<Utc>>,
}
