mod havana;
mod naive_monte_carlo;

use super::{BuildError, BuildFromJson, EngineError};
use crate::batch::{Batch, PointSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::any::Any;
use strum::{AsRefStr, Display};

use self::havana::HavanaSampler;
use self::naive_monte_carlo::NaiveMonteCarloSamplerAggregator;

pub type BatchContext = Box<dyn Any + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SamplerAggregatorImplementation {
    NaiveMonteCarlo,
    Havana,
}

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
    fn produce_batch(
        &mut self,
        nr_samples: usize,
    ) -> Result<(Batch, Option<BatchContext>), EngineError>;
    fn ingest_training_weights(
        &mut self,
        training_weights: &[f64],
        context: Option<BatchContext>,
    ) -> Result<(), EngineError>;
    fn get_diagnostics(&mut self) -> JsonValue {
        json!("{}")
    }
}

#[derive(Debug, Clone)]
pub struct SamplerAggregatorFactory {
    implementation: SamplerAggregatorImplementation,
    params: JsonValue,
}

impl SamplerAggregatorFactory {
    pub fn new(implementation: SamplerAggregatorImplementation, params: JsonValue) -> Self {
        Self {
            implementation,
            params,
        }
    }

    pub fn build(&self) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self.implementation {
            SamplerAggregatorImplementation::NaiveMonteCarlo => Ok(Box::new(
                NaiveMonteCarloSamplerAggregator::from_json(&self.params)?,
            )),
            SamplerAggregatorImplementation::Havana => {
                Ok(Box::new(HavanaSampler::from_json(&self.params)?))
            }
        }
    }
}
