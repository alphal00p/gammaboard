use crate::batch::{Batch, BatchResults};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{fmt, str::FromStr};

/// Immutable run configuration loaded from storage before starting a run loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: i32,
    pub evaluator_implementation: EvaluatorImplementation,
    pub evaluator_params: JsonValue,
    pub sampler_aggregator_implementation: SamplerAggregatorImplementation,
    pub sampler_aggregator_params: JsonValue,
    pub observable_implementation: ObservableImplementation,
    pub observable_params: JsonValue,
    pub worker_runner_params: JsonValue,
    pub sampler_aggregator_runner_params: JsonValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorImplementation {
    TestOnlySin,
}

impl EvaluatorImplementation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TestOnlySin => "test_only_sin",
        }
    }

    pub const fn version(self) -> &'static str {
        match self {
            Self::TestOnlySin => "v1",
        }
    }
}

impl fmt::Display for EvaluatorImplementation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplerAggregatorImplementation {
    TestOnlyTraining,
}

impl SamplerAggregatorImplementation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TestOnlyTraining => "test_only_training",
        }
    }

    pub const fn version(self) -> &'static str {
        match self {
            Self::TestOnlyTraining => "v1",
        }
    }
}

impl fmt::Display for SamplerAggregatorImplementation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservableImplementation {
    TestOnly,
}

impl ObservableImplementation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TestOnly => "test_only",
        }
    }

    pub const fn version(self) -> &'static str {
        match self {
            Self::TestOnly => "v1",
        }
    }
}

impl fmt::Display for ObservableImplementation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Canonical integration parameters payload stored on `runs.integration_params`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct IntegrationParams {
    pub evaluator_implementation: Option<EvaluatorImplementation>,
    pub evaluator_params: Option<JsonValue>,
    pub sampler_aggregator_implementation: Option<SamplerAggregatorImplementation>,
    pub sampler_aggregator_params: Option<JsonValue>,
    pub observable_implementation: Option<ObservableImplementation>,
    pub observable_params: Option<JsonValue>,
    pub worker_runner_params: Option<JsonValue>,
    pub sampler_aggregator_runner_params: Option<JsonValue>,
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
}

#[derive(Debug, Clone)]
pub struct CompletedBatch {
    pub batch_id: i64,
    pub batch: Batch,
    pub results: BatchResults,
    pub batch_observable: JsonValue,
    pub completed_at: Option<DateTime<Utc>>,
}
