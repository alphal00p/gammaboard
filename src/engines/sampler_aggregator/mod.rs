mod havana;
mod test_only_training;

use super::{BuildError, BuildFromJson, EngineError};
use crate::batch::{Batch, PointSpec};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::any::Any;
use strum::{AsRefStr, Display};

use self::havana::HavanaSampler;
use self::test_only_training::TestTrainingSamplerAggregator;

pub type BatchContext = Box<dyn Any + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SamplerAggregatorImplementation {
    TestOnlyTraining,
    Havana,
}

/// Owns adaptive sampling training for a single run.
#[enum_dispatch]
pub trait SamplerAggregator: Send {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn get_max_batches(&self) -> Option<usize> {
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

#[enum_dispatch(SamplerAggregator)]
pub enum SamplerAggregatorEngine {
    TestOnlyTraining(TestTrainingSamplerAggregator),
    Havana(HavanaSampler),
}

impl SamplerAggregatorEngine {
    pub fn build(
        implementation: SamplerAggregatorImplementation,
        params: &JsonValue,
    ) -> Result<Self, BuildError> {
        match implementation {
            SamplerAggregatorImplementation::TestOnlyTraining => Ok(Self::TestOnlyTraining(
                TestTrainingSamplerAggregator::from_json(params)?,
            )),
            SamplerAggregatorImplementation::Havana => {
                Ok(Self::Havana(HavanaSampler::from_json(params)?))
            }
        }
    }
}
