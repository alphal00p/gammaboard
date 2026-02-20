pub mod errors;
pub mod havana_sampler;
pub mod models;
pub mod scalar_observable;
pub mod test_only;
pub mod traits;

pub use errors::{BuildError, EngineError, EvalError};
pub use models::{
    EvaluatorImplementation, IntegrationParams, RunSpec, SamplerAggregatorImplementation,
};
pub use traits::{
    Evaluator, Observable, SamplerAggregatorEngine, decode_observable_state,
    encode_observable_state,
};

use havana_sampler::HavanaSampler;
use scalar_observable::ScalarObservableAggregator;
use serde_json::Value as JsonValue;
use test_only::{TestSinEvaluator, TestTrainingSamplerAggregator};

impl EvaluatorImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn Evaluator>, BuildError> {
        match self {
            Self::TestOnlySin => Ok(Box::new(TestSinEvaluator::from_params(params)?)),
        }
    }

    pub fn build_observable(self, params: &JsonValue) -> Result<Box<dyn Observable>, BuildError> {
        match self {
            Self::TestOnlySin => Ok(Box::new(ScalarObservableAggregator::from_params(params)?)),
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
