use crate::core::{EvalError, ObservableConfig};
use crate::evaluation::{
    Batch, BatchResult, EvalBatchOptions, Evaluator, IngestScalar, ObservableState, PointSpec,
};
use serde::{Deserialize, Serialize};
use std::{
    thread,
    time::{Duration, Instant},
};

/// Test-only evaluator used for local end-to-end runs.
pub struct SinEvaluator {
    min_eval_time_per_sample_ms: u64,
}

impl SinEvaluator {
    pub fn new(min_eval_time_per_sample_ms: u64) -> Self {
        Self {
            min_eval_time_per_sample_ms,
        }
    }

    pub fn from_params(params: SinEvaluatorParams) -> Self {
        Self::new(params.min_eval_time_per_sample_ms)
    }

    fn eval_scalar_into<O: IngestScalar>(
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
            let value = x.sin() * (-x * x).exp();
            observable.ingest_scalar(value, *weight);
            if let Some(values) = values.as_mut() {
                values.push(value * *weight);
            }
        }
        Ok(values)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SinEvaluatorParams {
    pub min_eval_time_per_sample_ms: u64,
}

impl Evaluator for SinEvaluator {
    fn get_point_spec(&self) -> PointSpec {
        PointSpec {
            continuous_dims: 1,
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
            ObservableState::Scalar(observable) => {
                self.eval_scalar_into(batch, observable, options.require_training_values)?
            }
            ObservableState::FullScalar(observable) => {
                self.eval_scalar_into(batch, observable, options.require_training_values)?
            }
            other => {
                return Err(EvalError::eval(format!(
                    "sin evaluator does not support observable kind {}",
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
