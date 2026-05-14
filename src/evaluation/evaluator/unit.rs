use crate::core::{AccumulatorConfig, EvalError};
use crate::evaluation::{
    AccumulatorState, Batch, BatchResult, EvalBatchOptions, Evaluator, IngestScalar,
    ScalarSampleEvaluator, SemanticAccumulatorKind,
};
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Evaluator that returns 1.0 for every sample.
pub struct UnitEvaluator {
    domain: Domain,
    accumulator_kind: SemanticAccumulatorKind,
    fail_on_batch_nr: Option<usize>,
    fail_on_batch_nrs: Vec<usize>,
    min_eval_time_per_sample_ms: u64,
    eval_batches_total: usize,
}

impl UnitEvaluator {
    pub fn new(
        domain: Domain,
        accumulator_kind: SemanticAccumulatorKind,
        fail_on_batch_nr: Option<usize>,
        fail_on_batch_nrs: Vec<usize>,
        min_eval_time_per_sample_ms: u64,
    ) -> Self {
        Self {
            domain,
            accumulator_kind,
            fail_on_batch_nr,
            fail_on_batch_nrs,
            min_eval_time_per_sample_ms,
            eval_batches_total: 0,
        }
    }

    pub fn from_params(params: UnitEvaluatorParams) -> Self {
        Self::new(
            Domain::rectangular(params.continuous_dims, params.discrete_dims),
            params.accumulator_kind,
            params.fail_on_batch_nr,
            params.fail_on_batch_nrs,
            params.min_eval_time_per_sample_ms,
        )
    }

    fn maybe_sleep(&self) {
        if self.min_eval_time_per_sample_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.min_eval_time_per_sample_ms));
        }
    }

    fn scalar_ingestor<'a>(
        state: &'a mut AccumulatorState,
    ) -> Result<&'a mut dyn IngestScalar, EvalError> {
        match state {
            AccumulatorState::Empty(accumulator) => Ok(accumulator),
            AccumulatorState::Scalar(accumulator) => Ok(accumulator),
            AccumulatorState::FullScalar(accumulator) => Ok(accumulator),
            other => Err(EvalError::eval(format!(
                "unit evaluator scalar mode does not support accumulator kind {}",
                other.kind_str()
            ))),
        }
    }

    fn should_fail_on_batch(&self, batch_nr: usize) -> bool {
        self.fail_on_batch_nr
            .is_some_and(|n| n > 0 && n == batch_nr)
            || self
                .fail_on_batch_nrs
                .iter()
                .any(|n| *n > 0 && *n == batch_nr)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UnitEvaluatorParams {
    pub continuous_dims: usize,
    pub discrete_dims: usize,
    pub accumulator_kind: SemanticAccumulatorKind,
    #[serde(default)]
    pub fail_on_batch_nr: Option<usize>,
    #[serde(default)]
    pub fail_on_batch_nrs: Vec<usize>,
    #[serde(default)]
    pub min_eval_time_per_sample_ms: u64,
}

impl Default for UnitEvaluatorParams {
    fn default() -> Self {
        Self {
            continuous_dims: 1,
            discrete_dims: 0,
            accumulator_kind: SemanticAccumulatorKind::Scalar,
            fail_on_batch_nr: None,
            fail_on_batch_nrs: Vec::new(),
            min_eval_time_per_sample_ms: 0,
        }
    }
}

impl ScalarSampleEvaluator for UnitEvaluator {
    fn eval_scalar_sample(&mut self, _batch: &Batch, _sample_idx: usize) -> Result<f64, EvalError> {
        self.maybe_sleep();
        Ok(1.0)
    }
}

impl Evaluator for UnitEvaluator {
    fn get_domain(&self) -> Domain {
        self.domain.clone()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        self.eval_batches_total = self.eval_batches_total.saturating_add(1);
        if self.should_fail_on_batch(self.eval_batches_total) {
            return Err(EvalError::eval(format!(
                "unit evaluator injected failure on batch {}",
                self.eval_batches_total
            )));
        }
        let mut observable_state = AccumulatorState::from_config(accumulator);
        if self.accumulator_kind != SemanticAccumulatorKind::Scalar {
            return Err(EvalError::eval(
                "unit evaluator only supports scalar accumulator_kind; use process_evaluator for vector outputs",
            ));
        }
        let weighted_values = self.eval_scalar_into(
            batch,
            Self::scalar_ingestor(&mut observable_state)?,
            options.require_training_values,
        )?;
        Ok(BatchResult::new(weighted_values, observable_state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{Batch, Point};

    #[test]
    fn eval_batch_returns_weighted_ones_for_scalar_observable() {
        let batch = Batch::from_points([
            Point::new(vec![0.0], Vec::new(), 2.0),
            Point::new(vec![1.0], Vec::new(), 3.0),
        ])
        .expect("batch");
        let mut evaluator = UnitEvaluator::new(
            Domain::continuous(1),
            SemanticAccumulatorKind::Scalar,
            None,
            Vec::new(),
            0,
        );

        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::scalar(),
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        let AccumulatorState::Scalar(accumulator) = result.accumulator else {
            panic!("expected scalar accumulator");
        };
        assert_eq!(accumulator.count, 2);
        assert_eq!(accumulator.sum_weighted_value, 5.0);
    }
}
