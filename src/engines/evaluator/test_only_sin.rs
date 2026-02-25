use crate::batch::{Batch, BatchResult, PointSpec};
use crate::engines::observable::ObservableFactory;
use crate::engines::{
    BuildError, BuildFromJson, EvalError, Evaluator, Observable, ObservableEngine,
    ObservableImplementation,
};
use serde::Deserialize;
use std::{
    thread,
    time::{Duration, Instant},
};

/// Test-only evaluator used for local end-to-end runs.
pub struct TestSinEvaluator {
    min_eval_time_per_sample_ms: u64,
}

impl TestSinEvaluator {
    pub fn new(min_eval_time_per_sample_ms: u64) -> Self {
        Self {
            min_eval_time_per_sample_ms,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TestEvaluatorParams {
    pub min_eval_time_per_sample_ms: u64,
}

impl Evaluator for TestSinEvaluator {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != 1 {
            return Err(BuildError::build(format!(
                "test_only_sin evaluator expects continuous_dims=1, got {}",
                point_spec.continuous_dims
            )));
        }
        if point_spec.discrete_dims != 0 {
            return Err(BuildError::build(format!(
                "test_only_sin evaluator expects discrete_dims=0, got {}",
                point_spec.discrete_dims
            )));
        }
        Ok(())
    }

    fn eval_batch(
        &self,
        batch: &Batch,
        observable_factory: &ObservableFactory,
    ) -> Result<BatchResult, EvalError> {
        let implementation = observable_factory.implementation;

        if implementation != ObservableImplementation::Scalar {
            return Err(EvalError::eval(format!(
                "test_only_sin supports only scalar observable, got {implementation}"
            )));
        }
        let mut observable = match observable_factory
            .build()
            .map_err(|err| EvalError::eval(err.to_string()))?
        {
            ObservableEngine::Scalar(scalar_observable_aggregator) => {
                Ok(scalar_observable_aggregator)
            }
            _ => Err(EvalError::eval(
                "test_only_sin supports only scalar observable",
            )),
        }?;
        let started = Instant::now();
        let mut values = Vec::with_capacity(batch.size());

        for (row, weight) in batch
            .continuous()
            .rows()
            .into_iter()
            .zip(batch.weights().iter())
        {
            let x = *row
                .get(0)
                .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
            let value = x.sin() * (-x * x).exp();
            observable.add_sample(value, *weight);
            values.push(value);
        }

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        let batch_observable = observable
            .snapshot()
            .map_err(|err| EvalError::eval(err.to_string()))?;

        Ok(BatchResult::new(values, batch_observable))
    }

    fn supports_observable(&self, observable: &ObservableEngine) -> bool {
        matches!(observable, ObservableEngine::Scalar(_))
    }
}

impl BuildFromJson for TestSinEvaluator {
    type Params = TestEvaluatorParams;
    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new(params.min_eval_time_per_sample_ms))
    }
}
