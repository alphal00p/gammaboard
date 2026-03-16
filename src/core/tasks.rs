use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::{ParametrizationConfig, SamplerAggregatorConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTaskState {
    Pending,
    Active,
    Completed,
    Failed,
}

impl RunTaskState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunTaskSpec {
    Sample {
        nr_samples: Option<i64>,
        sampler_aggregator: SamplerAggregatorConfig,
        parametrization: ParametrizationConfig,
    },
    Pause,
}

impl RunTaskSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Sample {
                nr_samples: Some(nr_samples),
                ..
            } if *nr_samples <= 0 => {
                Err("sample task nr_samples must be a positive integer when set".to_string())
            }
            _ => Ok(()),
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Sample { .. } => "sample",
            Self::Pause => "pause",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTask {
    pub id: i64,
    pub run_id: i32,
    pub sequence_nr: i32,
    pub task: RunTaskSpec,
    pub state: RunTaskState,
    pub nr_produced_samples: i64,
    pub nr_completed_samples: i64,
    pub failure_reason: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
