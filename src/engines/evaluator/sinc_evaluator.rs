use crate::batch::{Batch, BatchResult, PointSpec};
use crate::engines::EvalBatchOptions;
use crate::engines::evaluator::sin_evaluator::SinEvaluatorParams;
use crate::engines::observable::ObservableFactory;
use crate::engines::{BuildError, BuildFromJson, EvalError, Evaluator};
use num::complex::Complex64;
use std::{
    thread,
    time::{Duration, Instant},
};

/// Test-only evaluator used for local end-to-end runs.
pub struct SincEvaluator {
    min_eval_time_per_sample_ms: u64,
}

impl SincEvaluator {
    pub fn new(min_eval_time_per_sample_ms: u64) -> Self {
        Self {
            min_eval_time_per_sample_ms,
        }
    }
}

impl Evaluator for SincEvaluator {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != 2 {
            return Err(BuildError::build(format!(
                "sinc_evaluator expects continuous_dims=2, got {}",
                point_spec.continuous_dims
            )));
        }
        if point_spec.discrete_dims != 0 {
            return Err(BuildError::build(format!(
                "sinc_evaluator expects discrete_dims=0, got {}",
                point_spec.discrete_dims
            )));
        }
        Ok(())
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable_factory: &ObservableFactory,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let mut observable = observable_factory
            .build()
            .map_err(|err| EvalError::eval(err.to_string()))?;
        let started = Instant::now();
        let mut values = if options.require_training_values {
            Some(Vec::with_capacity(batch.size()))
        } else {
            None
        };

        {
            let complex_ingest = observable.as_complex_ingest().ok_or_else(|| {
                EvalError::eval(format!(
                    "sinc_evaluator supports only complex-capable observables, got {}",
                    observable_factory.implementation
                ))
            })?;
            for (row, weight) in batch.continuous().rows().into_iter().zip(weights.iter()) {
                let x = *row
                    .get(0)
                    .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
                let y = *row
                    .get(1)
                    .ok_or_else(|| EvalError::eval("missing continuous[1]"))?;
                let z = Complex64::new(x, y);
                let value = z.sin();
                complex_ingest.ingest_complex(value, *weight);
                if let Some(values) = values.as_mut() {
                    values.push(value.norm());
                }
            }
        }

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        if let Some(values) = values {
            BatchResult::from_values_weights_and_observable(values, weights, observable.as_ref())
        } else {
            BatchResult::from_observable_only(observable.as_ref())
        }
    }

    fn supports_observable(&self, observable_factory: &ObservableFactory) -> bool {
        match observable_factory.build() {
            Ok(mut observable) => observable.as_complex_ingest().is_some(),
            Err(_) => false,
        }
    }
}

impl BuildFromJson for SincEvaluator {
    type Params = SinEvaluatorParams;
    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new(params.min_eval_time_per_sample_ms))
    }
}
