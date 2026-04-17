use crate::core::{AccumulatorConfig, EvalError};
use crate::evaluation::{
    AccumulatorState, Batch, BatchResult, ComplexSampleEvaluator, EvalBatchOptions, Evaluator,
    ScalarSampleEvaluator, SemanticAccumulatorKind,
};
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};

/// Evaluator that returns 1.0 for every sample.
pub struct UnitEvaluator {
    domain: Domain,
    accumulator_kind: SemanticAccumulatorKind,
    fail_on_batch_nr: Option<usize>,
    eval_batches_total: usize,
}

impl UnitEvaluator {
    pub fn new(
        domain: Domain,
        accumulator_kind: SemanticAccumulatorKind,
        fail_on_batch_nr: Option<usize>,
    ) -> Self {
        Self {
            domain,
            accumulator_kind,
            fail_on_batch_nr,
            eval_batches_total: 0,
        }
    }

    pub fn from_params(params: UnitEvaluatorParams) -> Self {
        Self::new(
            Domain::rectangular(params.continuous_dims, params.discrete_dims),
            params.accumulator_kind,
            params.fail_on_batch_nr,
        )
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
}

impl Default for UnitEvaluatorParams {
    fn default() -> Self {
        Self {
            continuous_dims: 1,
            discrete_dims: 0,
            accumulator_kind: SemanticAccumulatorKind::Scalar,
            fail_on_batch_nr: None,
        }
    }
}

impl ScalarSampleEvaluator for UnitEvaluator {
    fn eval_scalar_sample(&mut self, _batch: &Batch, _sample_idx: usize) -> Result<f64, EvalError> {
        Ok(1.0)
    }
}

impl ComplexSampleEvaluator for UnitEvaluator {
    fn eval_complex_sample(
        &mut self,
        _batch: &Batch,
        _sample_idx: usize,
    ) -> Result<num::complex::Complex64, EvalError> {
        Ok(num::complex::Complex64::new(1.0, 0.0))
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
        if self
            .fail_on_batch_nr
            .is_some_and(|n| n > 0 && self.eval_batches_total == n)
        {
            return Err(EvalError::eval(format!(
                "unit evaluator injected failure on batch {}",
                self.eval_batches_total
            )));
        }
        let mut observable_state = AccumulatorState::from_config(accumulator);
        let weighted_values = match self.accumulator_kind {
            SemanticAccumulatorKind::Scalar => match &mut observable_state {
                AccumulatorState::Empty(accumulator) => {
                    self.eval_scalar_into(batch, accumulator, options.require_training_values)?
                }
                AccumulatorState::Scalar(accumulator) => {
                    self.eval_scalar_into(batch, accumulator, options.require_training_values)?
                }
                AccumulatorState::FullScalar(accumulator) => {
                    self.eval_scalar_into(batch, accumulator, options.require_training_values)?
                }
                other => {
                    return Err(EvalError::eval(format!(
                        "unit evaluator scalar mode does not support accumulator kind {}",
                        other.kind_str()
                    )));
                }
            },
            SemanticAccumulatorKind::Complex => match &mut observable_state {
                AccumulatorState::Empty(accumulator) => self.eval_complex_into(
                    batch,
                    accumulator,
                    options.require_training_values,
                    |v| v.re,
                )?,
                AccumulatorState::Complex(accumulator) => self.eval_complex_into(
                    batch,
                    accumulator,
                    options.require_training_values,
                    |v| v.re,
                )?,
                AccumulatorState::FullComplex(accumulator) => self.eval_complex_into(
                    batch,
                    accumulator,
                    options.require_training_values,
                    |v| v.re,
                )?,
                other => {
                    return Err(EvalError::eval(format!(
                        "unit evaluator complex mode does not support accumulator kind {}",
                        other.kind_str()
                    )));
                }
            },
        };
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
        let mut evaluator =
            UnitEvaluator::new(Domain::continuous(1), SemanticAccumulatorKind::Scalar, None);

        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::Scalar,
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

    #[test]
    fn eval_batch_supports_complex_accumulator_via_scalar_cast() {
        let batch = Batch::from_points([
            Point::new(vec![0.0], Vec::new(), 2.0),
            Point::new(vec![1.0], Vec::new(), 3.0),
        ])
        .expect("batch");
        let mut evaluator = UnitEvaluator::new(
            Domain::continuous(1),
            SemanticAccumulatorKind::Complex,
            None,
        );

        let result = evaluator
            .eval_batch(
                &batch,
                &AccumulatorConfig::Complex,
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        let AccumulatorState::Complex(accumulator) = result.accumulator else {
            panic!("expected complex accumulator");
        };
        assert_eq!(accumulator.count, 2);
        assert_eq!(accumulator.real_sum, 5.0);
        assert_eq!(accumulator.imag_sum, 0.0);
    }
}
