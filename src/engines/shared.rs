use super::BuildError;
use super::{EngineError, Observable};
use crate::core::{BatchResult, PointSpec};
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
    pub evaluator: EvaluatorConfig,
    pub sampler_aggregator: SamplerAggregatorConfig,
    pub observable: ObservableConfig,
    pub parametrization: ParametrizationConfig,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

/// Immutable run configuration loaded from storage before starting a run loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: i32,
    pub point_spec: PointSpec,
    pub evaluator: EvaluatorConfig,
    pub sampler_aggregator: SamplerAggregatorConfig,
    pub observable: ObservableConfig,
    pub parametrization: ParametrizationConfig,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluatorConfig {
    Gammaloop {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    SinEvaluator {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    SincEvaluator {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    Unit {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    Symbolica {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
}

impl EvaluatorConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SamplerAggregatorConfig {
    NaiveMonteCarlo {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    Havana {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
}

impl SamplerAggregatorConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservableConfig {
    Scalar {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    Complex {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
}

impl ObservableConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParametrizationConfig {
    None {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    Identity {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    UnitBall {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
    Spherical {
        #[serde(flatten)]
        params: serde_json::Map<String, JsonValue>,
    },
}

impl ParametrizationConfig {}
