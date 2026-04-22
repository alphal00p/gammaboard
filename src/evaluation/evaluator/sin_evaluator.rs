use crate::core::{AccumulatorConfig, EvalError};
use crate::evaluation::{
    AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator, IngestScalar,
    ScalarSampleEvaluator,
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

    fn scalar_ingestor<'a>(
        state: &'a mut AccumulatorState,
    ) -> Result<&'a mut dyn IngestScalar, EvalError> {
        match state {
            AccumulatorState::Empty(accumulator) => Ok(accumulator),
            AccumulatorState::Scalar(accumulator) => Ok(accumulator),
            AccumulatorState::FullScalar(accumulator) => Ok(accumulator),
            other => Err(EvalError::eval(format!(
                "sin evaluator does not support accumulator kind {}",
                other.kind_str()
            ))),
        }
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
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let started = Instant::now();
        let mut observable_state = AccumulatorState::from_config(accumulator);
        let weighted_values = self.eval_scalar_into(
            batch,
            Self::scalar_ingestor(&mut observable_state)?,
            options.require_training_values,
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
