use crate::batch::{Batch, BatchResult, PointSpec};
use crate::engines::evaluator::test_only_sin::TestEvaluatorParams;
use crate::engines::observable::ObservableFactory;
use crate::engines::{
    BuildError, BuildFromJson, EvalError, Evaluator, Observable, ObservableEngine,
};
use num::complex::Complex64;
use std::{
    thread,
    time::{Duration, Instant},
};

/// Test-only evaluator used for local end-to-end runs.
pub struct TestSincEvaluator {
    min_eval_time_per_sample_ms: u64,
}

impl TestSincEvaluator {
    pub fn new(min_eval_time_per_sample_ms: u64) -> Self {
        Self {
            min_eval_time_per_sample_ms,
        }
    }
}

impl Evaluator for TestSincEvaluator {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != 2 {
            return Err(BuildError::build(format!(
                "test_only_sinc evaluator expects continuous_dims=2, got {}",
                point_spec.continuous_dims
            )));
        }
        if point_spec.discrete_dims != 0 {
            return Err(BuildError::build(format!(
                "test_only_sinc evaluator expects discrete_dims=0, got {}",
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
        let mut observable = match observable_factory
            .build()
            .map_err(|err| EvalError::eval(err.to_string()))?
        {
            ObservableEngine::Complex(scalar_observable_aggregator) => {
                Ok(scalar_observable_aggregator)
            }
            _ => Err(EvalError::eval(
                "test_only_sinc supports only complex observable",
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
            let y = *row
                .get(1)
                .ok_or_else(|| EvalError::eval("missing continuous[1]"))?;
            let z = Complex64::new(x, y);
            let value = z.sin();
            observable.add_sample(value, *weight);
            values.push(value.norm());
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
        matches!(observable, ObservableEngine::Complex(_))
    }
}

impl BuildFromJson for TestSincEvaluator {
    type Params = TestEvaluatorParams;
    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new(params.min_eval_time_per_sample_ms))
    }
}
