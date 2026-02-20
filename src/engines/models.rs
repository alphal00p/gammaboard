use crate::batch::PointSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::fmt;

/// Immutable run configuration loaded from storage before starting a run loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: i32,
    pub point_spec: PointSpec,
    pub evaluator_implementation: EvaluatorImplementation,
    pub evaluator_params: JsonValue,
    pub sampler_aggregator_implementation: SamplerAggregatorImplementation,
    pub sampler_aggregator_params: JsonValue,
    pub observable_implementation: ObservableImplementation,
    pub observable_params: JsonValue,
    pub evaluator_runner_params: JsonValue,
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

    pub const fn supports_observable(self, observable: ObservableImplementation) -> bool {
        matches!(
            (self, observable),
            (Self::TestOnlySin, ObservableImplementation::Scalar)
        )
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
    Havana,
}

impl SamplerAggregatorImplementation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TestOnlyTraining => "test_only_training",
            Self::Havana => "havana",
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
    Scalar,
}

impl ObservableImplementation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
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
    pub evaluator_runner_params: Option<JsonValue>,
    pub sampler_aggregator_runner_params: Option<JsonValue>,
}
