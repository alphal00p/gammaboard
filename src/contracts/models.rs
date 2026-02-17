use crate::{Batch, BatchResults};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Immutable run configuration loaded from storage before starting a run loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: i32,
    pub evaluator_implementation: String,
    pub evaluator_version: String,
    pub sampler_aggregator_implementation: String,
    pub sampler_aggregator_version: String,
    pub integrand: JsonValue,
    pub integration_params: JsonValue,
}

/// In-memory state passed to a sampler-aggregator during initialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineState {
    pub last_processed_batch_id: Option<i64>,
    pub state: JsonValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentRole {
    Evaluator,
    SamplerAggregator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Active,
    Draining,
    Inactive,
}

/// Registered running process (role-specific identity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInstance {
    pub instance_id: String,
    pub node_id: Option<String>,
    pub role: ComponentRole,
    pub implementation: String,
    pub version: String,
    pub node_specs: JsonValue,
    pub status: InstanceStatus,
    pub last_seen: Option<DateTime<Utc>>,
}

/// Assignment lease for a run/component pairing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignmentLease {
    pub run_id: i32,
    pub role: ComponentRole,
    pub instance_id: String,
    pub lease_expires_at: DateTime<Utc>,
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
