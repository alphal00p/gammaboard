use crate::core::{EvalError, ObservableConfig};
use crate::evaluation::{
    Batch, BatchResult, EvalBatchOptions, Evaluator, ObservableState, ScalarSampleEvaluator,
};
use crate::utils::domain::Domain;
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

impl ScalarSampleEvaluator for SinEvaluator {
    fn eval_scalar_sample(&mut self, batch: &Batch, sample_idx: usize) -> Result<f64, EvalError> {
        let point = batch
            .point(sample_idx)
            .ok_or_else(|| EvalError::eval(format!("missing sample {sample_idx}")))?;
        let x = *point
            .continuous
            .first()
            .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
        Ok(x.sin() * (-x * x).exp())
    }
}

impl Evaluator for SinEvaluator {
    fn get_domain(&self) -> Domain {
        Domain::continuous(1)
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
