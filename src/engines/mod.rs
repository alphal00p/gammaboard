pub mod errors;
pub mod havana_sampler;
pub mod models;
pub mod test_only;
pub mod traits;

pub use errors::{BuildError, EngineError, EvalError};
pub use models::{
    EngineState, EvaluatorImplementation, IntegrationParams, ObservableImplementation, RunSpec,
    SamplerAggregatorImplementation,
};
pub use traits::{AggregatedObservable, Evaluator, SamplerAggregatorEngine};

use serde_json::Value as JsonValue;
use test_only::{
    TestOnlyObservableAggregatorFactory, TestOnlySinEvaluatorFactory,
    TestOnlyTrainingSamplerAggregatorFactory,
};

impl EvaluatorImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn Evaluator>, BuildError> {
        match self {
            Self::TestOnlySin => TestOnlySinEvaluatorFactory::build(params),
        }
    }
}

impl SamplerAggregatorImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn SamplerAggregatorEngine>, BuildError> {
        match self {
            Self::TestOnlyTraining => TestOnlyTrainingSamplerAggregatorFactory::build(params),
        }
    }
}

impl ObservableImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> {
        match self {
            Self::TestOnly => TestOnlyObservableAggregatorFactory::build(params),
        }
    }
}
