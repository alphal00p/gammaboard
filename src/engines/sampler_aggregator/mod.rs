mod havana;
mod naive_monte_carlo;

use super::{BuildError, EngineError};
use crate::core::{Batch, PointSpec};
use serde_json::{Value as JsonValue, json};

use self::havana::{HavanaSampler, HavanaSamplerParams};
use self::naive_monte_carlo::{NaiveMonteCarloSamplerAggregator, NaiveMonteCarloSamplerParams};

/// Owns adaptive sampling training for a single run.
pub trait SamplerAggregator: Send {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn get_init_metadata(&mut self) -> JsonValue {
        json!({})
    }
    fn is_training_active(&self) -> bool {
        true
    }
    fn get_max_samples(&self) -> Option<usize> {
        None
    }
    fn produce_batch(&mut self, nr_samples: usize) -> Result<Batch, EngineError>;
    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError>;
    fn get_diagnostics(&mut self) -> JsonValue {
        json!("{}")
    }
}

impl crate::engines::SamplerAggregatorConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::NaiveMonteCarlo { .. } => "naive_monte_carlo",
            Self::Havana { .. } => "havana",
        }
    }

    pub fn build(&self, point_spec: PointSpec) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { params } => {
                let params: NaiveMonteCarloSamplerParams =
                    serde_json::from_value(JsonValue::Object(params.clone()))
                        .map_err(|err| BuildError::invalid_input(err.to_string()))?;
                Ok(Box::new(
                    NaiveMonteCarloSamplerAggregator::from_params_and_point_spec(
                        params,
                        &point_spec,
                    )?,
                ))
            }
            Self::Havana { params } => {
                let params: HavanaSamplerParams =
                    serde_json::from_value(JsonValue::Object(params.clone()))
                        .map_err(|err| BuildError::invalid_input(err.to_string()))?;
                Ok(Box::new(HavanaSampler::from_params_and_point_spec(
                    params,
                    &point_spec,
                )?))
            }
        }
    }
}
