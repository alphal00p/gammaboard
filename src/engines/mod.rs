pub mod errors;
pub mod havana_sampler;
pub mod models;
pub mod test_only;
pub mod traits;

pub use errors::{BuildError, EngineError, EvalError};
pub use models::{
    EvaluatorImplementation, IntegrationParams, ObservableImplementation, RunSpec,
    SamplerAggregatorImplementation,
};
pub use traits::{AggregatedObservable, Evaluator, SamplerAggregatorEngine};

use havana_sampler::HavanaSampler;
use serde_json::Value as JsonValue;
use test_only::{TestObservableAggregator, TestSinEvaluator, TestTrainingSamplerAggregator};

impl EvaluatorImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn Evaluator>, BuildError> {
        match self {
            Self::TestOnlySin => Ok(Box::new(TestSinEvaluator::from_params(params)?)),
        }
    }
}

impl SamplerAggregatorImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn SamplerAggregatorEngine>, BuildError> {
        match self {
            Self::TestOnlyTraining => Ok(Box::new(TestTrainingSamplerAggregator::from_params(
                params,
            )?)),
            Self::Havana => Ok(Box::new(HavanaSampler::from_params(params)?)),
        }
    }
}

impl ObservableImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> {
        match self {
            Self::TestOnly => Ok(Box::new(TestObservableAggregator::from_params(params)?)),
        }
    }
}
