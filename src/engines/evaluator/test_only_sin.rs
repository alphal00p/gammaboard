use crate::batch::{Batch, BatchResult, PointSpec};
use crate::engines::observable::ObservableFactory;
use crate::engines::{BuildError, BuildFromJson, EvalError, Evaluator};
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
        &mut self,
        batch: &Batch,
        observable_factory: &ObservableFactory,
    ) -> Result<BatchResult, EvalError> {
        let mut observable = observable_factory
            .build()
            .map_err(|err| EvalError::eval(err.to_string()))?;
        let mut values = Vec::with_capacity(batch.size());
        let started = Instant::now();
        {
            let scalar_ingest = observable.as_scalar_ingest().ok_or_else(|| {
                EvalError::eval(format!(
                    "test_only_sin supports only scalar-capable observables, got {}",
                    observable_factory.implementation
                ))
            })?;
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
                scalar_ingest.ingest_scalar(value, *weight);
                values.push(value);
            }
        }

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        BatchResult::from_values_weights_and_observable(
            values,
            batch.weights().as_slice().expect("standard order"),
            observable.as_ref(),
        )
    }

    fn supports_observable(&self, observable_factory: &ObservableFactory) -> bool {
        match observable_factory.build() {
            Ok(mut observable) => observable.as_scalar_ingest().is_some(),
            Err(_) => false,
        }
    }
}

impl BuildFromJson for TestSinEvaluator {
    type Params = TestEvaluatorParams;
    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new(params.min_eval_time_per_sample_ms))
    }
}
