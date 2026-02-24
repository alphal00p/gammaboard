use super::BuildError;
use super::{EvaluatorImplementation, ObservableImplementation, SamplerAggregatorImplementation};
use crate::batch::PointSpec;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub trait BuildFromJson: Sized {
    type Params: DeserializeOwned;
    const PARAMS_CONTEXT: &'static str;

    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError>;

    fn from_json(params: &JsonValue) -> Result<Self, BuildError> {
        let parsed: Self::Params = serde_json::from_value(params.clone()).map_err(|err| {
            BuildError::build(format!("invalid {}: {}", Self::PARAMS_CONTEXT, err))
        })?;
        Self::from_parsed_params(parsed)
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
