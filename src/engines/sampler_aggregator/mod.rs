mod havana;
mod test_only_training;

use super::{BuildError, BuildFromJson, EngineError};
use crate::batch::{Batch, PointSpec};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::any::Any;
use std::fmt;

use self::havana::HavanaSampler;
use self::test_only_training::TestTrainingSamplerAggregator;

pub type BatchContext = Box<dyn Any + Send>;

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

/// Owns adaptive sampling training for a single run.
#[enum_dispatch]
pub trait SamplerAggregator: Send {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn init(&mut self) -> Result<(), EngineError>;
    fn produce_batch(
        &mut self,
        nr_samples: usize,
    ) -> Result<(Batch, Option<BatchContext>), EngineError>;
    fn ingest_training_weights(
        &mut self,
        training_weights: &[f64],
        context: Option<BatchContext>,
    ) -> Result<(), EngineError>;
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
