use crate::batch::{Batch, BatchResult, PointSpec};
use crate::engines::EvalBatchOptions;
use crate::engines::observable::ObservableFactory;
use crate::engines::{BuildError, BuildFromJson, EvalError, Evaluator};
use serde::Deserialize;

/// Evaluator that returns 1.0 for every sample.
pub struct UnitEvaluator;

impl UnitEvaluator {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct UnitEvaluatorParams {}

impl Evaluator for UnitEvaluator {
    fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
        Ok(())
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable_factory: &ObservableFactory,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let mut observable = observable_factory
            .build()
            .map_err(|err| EvalError::eval(err.to_string()))?;
        let values = vec![1.0; batch.size()];

        {
            let scalar_ingest = observable.as_scalar_ingest().ok_or_else(|| {
                EvalError::eval(format!(
                    "unit evaluator supports only scalar-capable observables, got {}",
                    observable_factory.implementation
                ))
            })?;
            for weight in weights.iter() {
                scalar_ingest.ingest_scalar(1.0, *weight);
            }
        }

        if options.require_training_values {
            BatchResult::from_values_weights_and_observable(values, weights, observable.as_ref())
        } else {
            BatchResult::from_observable_only(observable.as_ref())
        }
    }

    fn supports_observable(&self, observable_factory: &ObservableFactory) -> bool {
        match observable_factory.build() {
            Ok(mut observable) => observable.as_scalar_ingest().is_some(),
            Err(_) => false,
        }
    }
}

impl BuildFromJson for UnitEvaluator {
    type Params = UnitEvaluatorParams;

    fn from_parsed_params(_params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::Batch;
    use crate::engines::observable::ObservableImplementation;
    use serde_json::json;

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
        let observable_factory =
            ObservableFactory::new(ObservableImplementation::Scalar, json!({}));
        let mut evaluator = UnitEvaluator::new();

        let result = evaluator
            .eval_batch(
                &batch,
                &observable_factory,
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        assert_eq!(result.observable["count"], json!(2));
        assert_eq!(result.observable["sum_weight"], json!(5.0));
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
        let observable_factory =
            ObservableFactory::new(ObservableImplementation::Complex, json!({}));
        let mut evaluator = UnitEvaluator::new();

        assert!(evaluator.supports_observable(&observable_factory));
        let result = evaluator
            .eval_batch(
                &batch,
                &observable_factory,
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        assert_eq!(result.observable["count"], json!(2));
        assert_eq!(result.observable["real_sum"], json!(5.0));
        assert_eq!(result.observable["imag_sum"], json!(0.0));
    }
}
