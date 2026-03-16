use crate::core::EvalError;
use crate::evaluation::{
    Batch, BatchResult, EvalBatchOptions, Evaluator, ObservableState, PointSpec,
    ScalarObservableState,
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

    fn empty_observable(&self) -> ObservableState {
        ObservableState::empty_scalar()
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
        let mut observable = ScalarObservableState::default();
        let mut values = if options.require_training_values {
            Some(Vec::with_capacity(batch.size()))
        } else {
            None
        };
        let started = Instant::now();
        for (row, weight) in batch.continuous().rows().into_iter().zip(weights.iter()) {
            let x = *row
                .get(0)
                .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
            let value = x.sin() * (-x * x).exp();
            observable.add_sample(value, *weight);
            if let Some(values) = values.as_mut() {
                values.push(value);
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
            ObservableState::Scalar(observable),
        ))
    }
}
