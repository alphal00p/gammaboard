use crate::core::{AccumulatorConfig, EvalError};
use crate::evaluation::{
    AccumulatorState, Batch, BatchResult, ComplexSampleEvaluator, EvalBatchOptions, Evaluator,
    IngestComplex, SinEvaluatorParams,
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

    fn complex_ingestor<'a>(
        state: &'a mut AccumulatorState,
    ) -> Result<&'a mut dyn IngestComplex, EvalError> {
        match state {
            AccumulatorState::Empty(accumulator) => Ok(accumulator),
            AccumulatorState::Complex(accumulator) => Ok(accumulator),
            AccumulatorState::FullComplex(accumulator) => Ok(accumulator),
            other => Err(EvalError::eval(format!(
                "sinc evaluator does not support accumulator kind {}",
                other.kind_str()
            ))),
        }
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
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let started = Instant::now();
        let mut observable_state = AccumulatorState::from_config(accumulator);
        let weighted_values = self.eval_complex_into(
            batch,
            Self::complex_ingestor(&mut observable_state)?,
            options.require_training_values,
            |v| v.norm(),
        )?;

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        Ok(BatchResult::new(weighted_values, observable_state))
    }
}
