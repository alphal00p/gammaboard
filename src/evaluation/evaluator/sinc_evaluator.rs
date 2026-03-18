use crate::core::{EvalError, ObservableConfig};
use crate::evaluation::{
    Batch, BatchResult, EvalBatchOptions, Evaluator, IngestComplex, ObservableState, PointSpec,
    SinEvaluatorParams,
};
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

    fn eval_complex_into<O: IngestComplex>(
        &self,
        batch: &Batch,
        observable: &mut O,
        capture_training_values: bool,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let mut values = capture_training_values.then(|| Vec::with_capacity(batch.size()));

        for (row, weight) in batch.continuous().rows().into_iter().zip(weights.iter()) {
            let x = *row
                .get(0)
                .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
            let y = *row
                .get(1)
                .ok_or_else(|| EvalError::eval("missing continuous[1]"))?;
            let value = Complex64::new(x, y).sin();
            observable.ingest_complex(value, *weight);
            if let Some(values) = values.as_mut() {
                values.push(value.norm() * *weight);
            }
        }

        Ok(values)
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

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let started = Instant::now();
        let mut observable_state = ObservableState::from_config(observable);
        let weighted_values = match &mut observable_state {
            ObservableState::Complex(observable) => {
                self.eval_complex_into(batch, observable, options.require_training_values)?
            }
            ObservableState::FullComplex(observable) => {
                self.eval_complex_into(batch, observable, options.require_training_values)?
            }
            other => {
                return Err(EvalError::eval(format!(
                    "sinc evaluator does not support observable kind {}",
                    other.kind_str()
                )));
            }
        };

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        Ok(BatchResult::new(weighted_values, observable_state))
    }
}
