use crate::core::{Batch, BatchResult, PointSpec};
use crate::engines::EvalBatchOptions;
use crate::engines::{BuildError, BuildFromJson, EvalError, Evaluator, ObservableConfig};
use serde::Deserialize;
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
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SinEvaluatorParams {
    pub min_eval_time_per_sample_ms: u64,
}

impl Evaluator for SinEvaluator {
    fn get_point_spec(&self) -> PointSpec {
        PointSpec {
            continuous_dims: 1,
            discrete_dims: 0,
        }
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
        let mut values = if options.require_training_values {
            Some(Vec::with_capacity(batch.size()))
        } else {
            None
        };
        let started = Instant::now();
        {
            let scalar_ingest = observable.as_scalar_ingest().ok_or_else(|| {
                EvalError::eval(format!(
                    "sin_evaluator supports only scalar-capable observables, got {}",
                    observable_config.kind_str()
                ))
            })?;
            for (row, weight) in batch.continuous().rows().into_iter().zip(weights.iter()) {
                let x = *row
                    .get(0)
                    .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
                let value = x.sin() * (-x * x).exp();
                scalar_ingest.ingest_scalar(value, *weight);
                if let Some(values) = values.as_mut() {
                    values.push(value);
                }
            }
        }

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        if let Some(values) = values {
            BatchResult::from_values_weights_and_observable(values, weights, observable.as_ref())
        } else {
            BatchResult::from_observable_only(observable.as_ref())
        }
    }
}

impl BuildFromJson for SinEvaluator {
    type Params = SinEvaluatorParams;
    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new(params.min_eval_time_per_sample_ms))
    }
}
