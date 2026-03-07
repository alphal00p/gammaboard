use crate::core::{Batch, BatchResult, PointSpec};
use crate::engines::EvalBatchOptions;
use crate::engines::{
    BuildError, BuildFromJson, EvalError, Evaluator, ObservableState, ScalarObservableState,
    SemanticObservableKind,
};
use serde::Deserialize;

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
}

#[derive(Debug, Clone, Deserialize)]
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

    fn empty_observable(&self) -> ObservableState {
        self.observable_kind.empty_state()
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
        let values = vec![1.0; batch.size()];
        let observable = match self.observable_kind {
            SemanticObservableKind::Scalar => {
                let mut observable = ScalarObservableState::default();
                for weight in weights.iter().copied() {
                    observable.add_sample(1.0, weight);
                }
                ObservableState::Scalar(observable)
            }
            SemanticObservableKind::Complex => {
                let mut observable = crate::engines::ComplexObservableState::default();
                for weight in weights.iter().copied() {
                    observable.add_sample(num::complex::Complex64::new(1.0, 0.0), weight);
                }
                ObservableState::Complex(observable)
            }
        };

        let weighted_values = options.require_training_values.then(|| {
            values
                .into_iter()
                .zip(weights.iter().copied())
                .map(|(value, weight)| value * weight)
                .collect()
        });
        Ok(BatchResult::new(weighted_values, observable))
    }
}

impl BuildFromJson for UnitEvaluator {
    type Params = UnitEvaluatorParams;

    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new(
            PointSpec {
                continuous_dims: params.continuous_dims,
                discrete_dims: params.discrete_dims,
            },
            params.observable_kind,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Batch;

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
