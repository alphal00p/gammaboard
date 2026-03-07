use crate::core::{Batch, BatchResult, PointSpec};
use crate::engines::EvalBatchOptions;
use crate::engines::{BuildError, BuildFromJson, EvalError, Evaluator, ObservableConfig};
use serde::Deserialize;

/// Evaluator that returns 1.0 for every sample.
pub struct UnitEvaluator {
    point_spec: PointSpec,
}

impl UnitEvaluator {
    pub fn new(point_spec: PointSpec) -> Self {
        Self { point_spec }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UnitEvaluatorParams {
    pub continuous_dims: usize,
    pub discrete_dims: usize,
}

impl Default for UnitEvaluatorParams {
    fn default() -> Self {
        Self {
            continuous_dims: 1,
            discrete_dims: 0,
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
        observable_config: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let mut observable = observable_config
            .build()
            .map_err(|err| EvalError::eval(err.to_string()))?;
        let values = vec![1.0; batch.size()];

        {
            let scalar_ingest = observable.as_scalar_ingest().ok_or_else(|| {
                EvalError::eval(format!(
                    "unit evaluator supports only scalar-capable observables, got {}",
                    observable_config.kind_str()
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
}

impl BuildFromJson for UnitEvaluator {
    type Params = UnitEvaluatorParams;

    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new(PointSpec {
            continuous_dims: params.continuous_dims,
            discrete_dims: params.discrete_dims,
        }))
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
        let observable_config = ObservableConfig::Scalar {
            params: serde_json::Map::new(),
        };
        let mut evaluator = UnitEvaluator::new(PointSpec {
            continuous_dims: 1,
            discrete_dims: 0,
        });

        let result = evaluator
            .eval_batch(
                &batch,
                &observable_config,
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        assert_eq!(result.observable["count"], serde_json::json!(2));
        assert_eq!(
            result.observable["sum_weighted_value"],
            serde_json::json!(5.0)
        );
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
        let observable_config = ObservableConfig::Complex {
            params: serde_json::Map::new(),
        };
        let mut evaluator = UnitEvaluator::new(PointSpec {
            continuous_dims: 1,
            discrete_dims: 0,
        });

        let result = evaluator
            .eval_batch(
                &batch,
                &observable_config,
                EvalBatchOptions {
                    require_training_values: true,
                },
            )
            .expect("result");

        assert_eq!(result.values, Some(vec![2.0, 3.0]));
        assert_eq!(result.observable["count"], serde_json::json!(2));
        assert_eq!(result.observable["real_sum"], serde_json::json!(5.0));
        assert_eq!(result.observable["imag_sum"], serde_json::json!(0.0));
    }
}
