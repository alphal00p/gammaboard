pub mod errors;
pub mod havana_sampler;
pub mod models;
pub mod scalar_observable;
pub mod test_only;
pub mod traits;

pub use errors::{BuildError, EngineError, EvalError};
pub use models::{
    EvaluatorImplementation, IntegrationParams, ObservableImplementation, RunSpec,
    SamplerAggregatorImplementation,
};
pub use traits::{
    Evaluator, Observable, SamplerAggregatorEngine, decode_observable_state,
    encode_observable_state,
};

use havana_sampler::HavanaSampler;
use scalar_observable::ScalarObservableAggregator;
use serde_json::Value as JsonValue;
use test_only::{TestSinEvaluator, TestTrainingSamplerAggregator};

pub enum EvaluatorEngine {
    TestOnlySin(TestSinEvaluator),
}

impl EvaluatorEngine {
    pub fn build(
        implementation: EvaluatorImplementation,
        params: &JsonValue,
    ) -> Result<Self, BuildError> {
        match implementation {
            EvaluatorImplementation::TestOnlySin => {
                Ok(Self::TestOnlySin(TestSinEvaluator::from_params(params)?))
            }
        }
    }
}

impl Evaluator for EvaluatorEngine {
    fn validate_point_spec(&self, point_spec: &crate::batch::PointSpec) -> Result<(), BuildError> {
        match self {
            Self::TestOnlySin(engine) => engine.validate_point_spec(point_spec),
        }
    }

    fn eval_batch(
        &self,
        batch: &crate::batch::Batch,
        observable: &mut dyn Observable,
    ) -> Result<crate::batch::BatchResult, EvalError> {
        match self {
            Self::TestOnlySin(engine) => engine.eval_batch(batch, observable),
        }
    }
}

pub enum ObservableEngine {
    Scalar(ScalarObservableAggregator),
}

impl ObservableEngine {
    pub fn build(
        implementation: ObservableImplementation,
        params: &JsonValue,
    ) -> Result<Self, BuildError> {
        match implementation {
            ObservableImplementation::Scalar => Ok(Self::Scalar(
                ScalarObservableAggregator::from_params(params)?,
            )),
        }
    }
}

impl Observable for ObservableEngine {
    fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
        match self {
            Self::Scalar(engine) => engine.load_state_from_json(state),
        }
    }

    fn merge_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
        match self {
            Self::Scalar(engine) => engine.merge_state_from_json(state),
        }
    }

    fn snapshot(&self) -> Result<JsonValue, EngineError> {
        match self {
            Self::Scalar(engine) => engine.snapshot(),
        }
    }
}

pub enum SamplerAggregatorImpl {
    TestOnlyTraining(TestTrainingSamplerAggregator),
    Havana(HavanaSampler),
}

impl SamplerAggregatorImpl {
    pub fn build(
        implementation: SamplerAggregatorImplementation,
        params: &JsonValue,
    ) -> Result<Self, BuildError> {
        match implementation {
            SamplerAggregatorImplementation::TestOnlyTraining => Ok(Self::TestOnlyTraining(
                TestTrainingSamplerAggregator::from_params(params)?,
            )),
            SamplerAggregatorImplementation::Havana => {
                Ok(Self::Havana(HavanaSampler::from_params(params)?))
            }
        }
    }
}

impl SamplerAggregatorEngine for SamplerAggregatorImpl {
    fn validate_point_spec(&self, point_spec: &crate::batch::PointSpec) -> Result<(), BuildError> {
        match self {
            Self::TestOnlyTraining(engine) => engine.validate_point_spec(point_spec),
            Self::Havana(engine) => engine.validate_point_spec(point_spec),
        }
    }

    fn init(&mut self) -> Result<(), EngineError> {
        match self {
            Self::TestOnlyTraining(engine) => engine.init(),
            Self::Havana(engine) => engine.init(),
        }
    }

    fn produce_batches(
        &mut self,
        max_batches: usize,
    ) -> Result<Vec<crate::batch::Batch>, EngineError> {
        match self {
            Self::TestOnlyTraining(engine) => engine.produce_batches(max_batches),
            Self::Havana(engine) => engine.produce_batches(max_batches),
        }
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        match self {
            Self::TestOnlyTraining(engine) => engine.ingest_training_weights(training_weights),
            Self::Havana(engine) => engine.ingest_training_weights(training_weights),
        }
    }
}
