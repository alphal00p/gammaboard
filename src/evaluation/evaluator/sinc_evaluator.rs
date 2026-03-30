use crate::core::{EvalError, ObservableConfig};
use crate::evaluation::{
    Batch, BatchResult, ComplexSampleEvaluator, EvalBatchOptions, Evaluator, ObservableState,
    SinEvaluatorParams,
};
use crate::utils::domain::Domain;
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

impl ComplexSampleEvaluator for SincEvaluator {
    fn eval_complex_sample(
        &mut self,
        batch: &Batch,
        sample_idx: usize,
    ) -> Result<Complex64, EvalError> {
        let point = batch
            .point(sample_idx)
            .ok_or_else(|| EvalError::eval(format!("missing sample {sample_idx}")))?;
        let x = *point
            .continuous
            .first()
            .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
        let y = *point
            .continuous
            .get(1)
            .ok_or_else(|| EvalError::eval("missing continuous[1]"))?;
        Ok(Complex64::new(x, y).sin())
    }
}

impl Evaluator for SincEvaluator {
    fn get_domain(&self) -> Domain {
        Domain::continuous(2)
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
                self.eval_complex_into(batch, observable, options.require_training_values, |v| {
                    v.norm()
                })?
            }
            ObservableState::FullComplex(observable) => {
                self.eval_complex_into(batch, observable, options.require_training_values, |v| {
                    v.norm()
                })?
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
