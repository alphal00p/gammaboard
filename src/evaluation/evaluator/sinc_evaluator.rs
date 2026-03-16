use crate::engines::{
    Batch, BatchResult, ComplexObservableState, EvalBatchOptions, EvalError, Evaluator,
    ObservableState, PointSpec,
};
use crate::evaluation::SinEvaluatorParams;
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

    pub fn from_params(params: SincEvaluatorParams) -> Self {
        Self::new(params.min_eval_time_per_sample_ms)
    }
}

pub type SincEvaluatorParams = SinEvaluatorParams;

impl Evaluator for SincEvaluator {
    fn get_point_spec(&self) -> PointSpec {
        PointSpec {
            continuous_dims: 2,
            discrete_dims: 0,
        }
    }

    fn empty_observable(&self) -> ObservableState {
        ObservableState::empty_complex()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let mut observable = ComplexObservableState::default();
        let started = Instant::now();
        let mut values = if options.require_training_values {
            Some(Vec::with_capacity(batch.size()))
        } else {
            None
        };

        for (row, weight) in batch.continuous().rows().into_iter().zip(weights.iter()) {
            let x = *row
                .get(0)
                .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
            let y = *row
                .get(1)
                .ok_or_else(|| EvalError::eval("missing continuous[1]"))?;
            let z = Complex64::new(x, y);
            let value = z.sin();
            observable.add_sample(value, *weight);
            if let Some(values) = values.as_mut() {
                values.push(value.norm());
            }
        }

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        let weighted_values = values.map(|values| {
            values
                .into_iter()
                .zip(weights.iter().copied())
                .map(|(value, weight)| value * weight)
                .collect()
        });
        Ok(BatchResult::new(
            weighted_values,
            ObservableState::Complex(observable),
        ))
    }
}
