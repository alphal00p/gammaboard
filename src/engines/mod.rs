pub mod test_only;

use crate::contracts::{
    AggregatedObservable, AggregatedObservableFactory, BuildError, Evaluator,
    EvaluatorImplementation, ObservableImplementation, SamplerAggregatorEngine,
    SamplerAggregatorImplementation,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;
use test_only::{
    TestOnlyObservableAggregator, TestOnlySinEvaluator, TestOnlyTrainingSamplerAggregatorEngine,
};

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct TestOnlyEvaluatorParams {
    min_eval_time_per_sample_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct TestOnlySamplerAggregatorParams {
    batch_size: usize,
    training_target_samples: usize,
    training_delay_per_sample_ms: u64,
}

impl Default for TestOnlySamplerAggregatorParams {
    fn default() -> Self {
        Self {
            batch_size: 64,
            training_target_samples: 0,
            training_delay_per_sample_ms: 0,
        }
    }
}

fn parse_params<T: DeserializeOwned>(params: &JsonValue, component: &str) -> Result<T, BuildError> {
    serde_json::from_value(params.clone()).map_err(|err| {
        BuildError::build(format!("invalid params for component {component}: {err}"))
    })
}

impl EvaluatorImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn Evaluator>, BuildError> {
        match self {
            Self::TestOnlySin => {
                let parsed: TestOnlyEvaluatorParams = parse_params(params, self.as_str())?;
                Ok(Box::new(TestOnlySinEvaluator::new(
                    parsed.min_eval_time_per_sample_ms,
                )))
            }
        }
    }
}

impl SamplerAggregatorImplementation {
    pub fn build(self, params: &JsonValue) -> Result<Box<dyn SamplerAggregatorEngine>, BuildError> {
        match self {
            Self::TestOnlyTraining => {
                let parsed: TestOnlySamplerAggregatorParams = parse_params(params, self.as_str())?;
                Ok(Box::new(TestOnlyTrainingSamplerAggregatorEngine::new(
                    parsed.batch_size,
                    parsed.training_target_samples,
                    parsed.training_delay_per_sample_ms,
                )))
            }
        }
    }
}

impl ObservableImplementation {
    pub fn build(self, _params: &JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> {
        match self {
            Self::TestOnly => Ok(Box::new(TestOnlyObservableAggregator::new())),
        }
    }
}

impl AggregatedObservableFactory for ObservableImplementation {
    fn implementation(&self) -> &'static str {
        ObservableImplementation::as_str(*self)
    }

    fn version(&self) -> &'static str {
        ObservableImplementation::version(*self)
    }

    fn build(&self, params: &JsonValue) -> Result<Box<dyn AggregatedObservable>, BuildError> {
        (*self).build(params)
    }
}
