use super::BuildError;
use super::{
    EngineError, EvaluatorImplementation, Observable, ObservableImplementation,
    ParametrizationImplementation, SamplerAggregatorImplementation,
};
use crate::batch::{BatchResult, PointSpec};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub trait BuildFromJson: Sized {
    type Params: DeserializeOwned;

    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError>;

    fn from_json(params: &JsonValue) -> Result<Self, BuildError> {
        let parsed: Self::Params = serde_json::from_value(params.clone())
            .map_err(|err| BuildError::invalid_input(err.to_string()))?;
        Self::from_parsed_params(parsed)
    }
}

impl BatchResult {
    pub fn from_values_weights_and_observable<O: Observable + ?Sized>(
        values: Vec<f64>,
        weights: &[f64],
        observable: &O,
    ) -> Result<Self, EngineError> {
        if values.len() != weights.len() {
            return Err(EngineError::engine(format!(
                "result length mismatch: values has {}, weights has {}",
                values.len(),
                weights.len()
            )));
        }

        let weighted_values = values
            .into_iter()
            .zip(weights.iter().copied())
            .map(|(value, weight)| value * weight)
            .collect();
        let batch_observable = observable.snapshot()?;

        Ok(Self::new(Some(weighted_values), batch_observable))
    }

    pub fn from_observable_only<O: Observable + ?Sized>(
        observable: &O,
    ) -> Result<Self, EngineError> {
        let batch_observable = observable.snapshot()?;
        Ok(Self::new(None, batch_observable))
    }
}

/// Canonical integration parameters payload stored on `runs.integration_params`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationParams {
    pub evaluator_implementation: Option<EvaluatorImplementation>,
    pub evaluator_params: Option<JsonValue>,
    pub sampler_aggregator_implementation: Option<SamplerAggregatorImplementation>,
    pub sampler_aggregator_params: Option<JsonValue>,
    pub observable_implementation: Option<ObservableImplementation>,
    pub observable_params: Option<JsonValue>,
    pub parametrization_implementation: Option<ParametrizationImplementation>,
    pub parametrization_params: Option<JsonValue>,
    pub evaluator_runner_params: Option<EvaluatorRunnerParams>,
    pub sampler_aggregator_runner_params: Option<SamplerAggregatorRunnerParams>,
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
    pub parametrization_implementation: ParametrizationImplementation,
    pub parametrization_params: JsonValue,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}
