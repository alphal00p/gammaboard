use crate::batch::{Batch, BatchResult};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{fmt, str::FromStr};

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
    pub result: BatchResult,
    pub completed_at: Option<DateTime<Utc>>,
}
