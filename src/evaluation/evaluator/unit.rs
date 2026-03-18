use crate::core::{EvalError, ObservableConfig};
use crate::evaluation::{
    Batch, BatchResult, EvalBatchOptions, Evaluator, IngestComplex, IngestScalar, ObservableState,
    PointSpec, SemanticObservableKind,
};
use serde::{Deserialize, Serialize};

/// Evaluator that returns 1.0 for every sample.
pub struct UnitEvaluator {
    point_spec: PointSpec,
    observable_kind: SemanticObservableKind,
}

impl UnitEvaluator {
    pub fn new(point_spec: PointSpec, observable_kind: SemanticObservableKind) -> Self {
        Self {
            point_spec,
            observable_kind,
        }
    }

    pub fn from_params(params: UnitEvaluatorParams) -> Self {
        Self::new(
            PointSpec {
                continuous_dims: params.continuous_dims,
                discrete_dims: params.discrete_dims,
            },
            params.observable_kind,
        )
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
        for weight in weights.iter().copied() {
            observable.ingest_scalar(1.0, weight);
        }
        Ok(capture_training_values.then(|| weights.iter().copied().collect()))
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
        for weight in weights.iter().copied() {
            observable.ingest_complex(num::complex::Complex64::new(1.0, 0.0), weight);
        }
        Ok(capture_training_values.then(|| weights.iter().copied().collect()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UnitEvaluatorParams {
    pub continuous_dims: usize,
    pub discrete_dims: usize,
    pub observable_kind: SemanticObservableKind,
}

impl Default for UnitEvaluatorParams {
    fn default() -> Self {
        Self {
            continuous_dims: 1,
            discrete_dims: 0,
            observable_kind: SemanticObservableKind::Scalar,
        }
    }
}

impl Evaluator for UnitEvaluator {
    fn get_point_spec(&self) -> PointSpec {
        self.point_spec.clone()
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let mut observable_state = ObservableState::from_config(observable);
        let weighted_values = match self.observable_kind {
            SemanticObservableKind::Scalar => match &mut observable_state {
                ObservableState::Scalar(observable) => {
                    self.eval_scalar_into(batch, observable, options.require_training_values)?
                }
                ObservableState::FullScalar(observable) => {
                    self.eval_scalar_into(batch, observable, options.require_training_values)?
                }
                other => {
                    return Err(EvalError::eval(format!(
                        "unit evaluator scalar mode does not support observable kind {}",
                        other.kind_str()
                    )));
                }
            },
            SemanticObservableKind::Complex => match &mut observable_state {
                ObservableState::Complex(observable) => {
                    self.eval_complex_into(batch, observable, options.require_training_values)?
                }
                ObservableState::FullComplex(observable) => {
                    self.eval_complex_into(batch, observable, options.require_training_values)?
                }
                other => {
                    return Err(EvalError::eval(format!(
                        "unit evaluator complex mode does not support observable kind {}",
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
    use crate::evaluation::Batch;

    #[test]
    fn eval_batch_returns_weighted_ones_for_scalar_observable() {
        let batch = Batch::from_flat_data_with_weights(
            2,
            1,
            0,
            vec![0.0, 1.0],
            vec![],
            Some(vec![2.0, 3.0]),
        )
        .expect("batch");
        let mut evaluator = UnitEvaluator::new(
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            SemanticObservableKind::Scalar,
        );

        let result = evaluator
            .eval_batch(
                &batch,
                &ObservableConfig::Scalar,
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        let ObservableState::Scalar(observable) = result.observable else {
            panic!("expected scalar observable");
        };
        assert_eq!(observable.count, 2);
        assert_eq!(observable.sum_weighted_value, 5.0);
    }

    #[test]
    fn eval_batch_supports_complex_observable_via_scalar_cast() {
        let batch = Batch::from_flat_data_with_weights(
            2,
            1,
            0,
            vec![0.0, 1.0],
            vec![],
            Some(vec![2.0, 3.0]),
        )
        .expect("batch");
        let mut evaluator = UnitEvaluator::new(
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0,
            },
            SemanticObservableKind::Complex,
        );

        let result = evaluator
            .eval_batch(
                &batch,
                &ObservableConfig::Complex,
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        let ObservableState::Complex(observable) = result.observable else {
            panic!("expected complex observable");
        };
        assert_eq!(observable.count, 2);
        assert_eq!(observable.real_sum, 5.0);
        assert_eq!(observable.imag_sum, 0.0);
    }
}
