//! Test-only runtime implementations used by local control-plane smoke tests.

use crate::batch::{Batch, BatchResult, PointSpec};
use crate::engines::{
    BuildError, EngineError, EvalError, Evaluator, Observable, SamplerAggregatorEngine,
    encode_observable_state,
};
use rand::Rng;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::{
    thread,
    time::{Duration, Instant},
};

/// Test-only evaluator used for local end-to-end runs.
pub struct TestSinEvaluator {
    min_eval_time_per_sample_ms: u64,
}

impl TestSinEvaluator {
    pub fn new(min_eval_time_per_sample_ms: u64) -> Self {
        Self {
            min_eval_time_per_sample_ms,
        }
    }

    pub fn from_params(params: &JsonValue) -> Result<Self, BuildError> {
        let parsed: TestEvaluatorParams = serde_json::from_value(params.clone())
            .map_err(|err| BuildError::build(format!("invalid evaluator params: {err}")))?;
        Ok(Self::new(parsed.min_eval_time_per_sample_ms))
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TestEvaluatorParams {
    min_eval_time_per_sample_ms: u64,
}

impl Evaluator for TestSinEvaluator {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != 1 {
            return Err(BuildError::build(format!(
                "test_only_sin evaluator expects continuous_dims=1, got {}",
                point_spec.continuous_dims
            )));
        }
        if point_spec.discrete_dims != 0 {
            return Err(BuildError::build(format!(
                "test_only_sin evaluator expects discrete_dims=0, got {}",
                point_spec.discrete_dims
            )));
        }
        Ok(())
    }

    fn eval_batch(
        &self,
        batch: &Batch,
        observable: &mut dyn Observable,
    ) -> Result<BatchResult, EvalError> {
        let started = Instant::now();
        let mut values = Vec::with_capacity(batch.size());

        for row in batch.continuous().rows() {
            let x = *row
                .get(0)
                .ok_or_else(|| EvalError::eval("missing continuous[0]"))?;
            let value = x.sin() * (-x * x).exp();
            values.push(value);
        }

        let min_total =
            Duration::from_millis(self.min_eval_time_per_sample_ms).mul_f64(batch.size() as f64);
        let elapsed = started.elapsed();
        if elapsed < min_total {
            thread::sleep(min_total - elapsed);
        }

        let count = values.len() as i64;
        let sum = values.iter().sum::<f64>();
        let sum_abs = values.iter().map(|v| v.abs()).sum::<f64>();
        let sum_sq = values.iter().map(|v| v * v).sum::<f64>();
        let delta = encode_observable_state(
            &serde_json::json!({
                "count": count,
                "sum": sum,
                "sum_abs": sum_abs,
                "sum_sq": sum_sq,
            }),
            "test batch scalar observable",
        )
        .map_err(|err| EvalError::eval(err.to_string()))?;
        observable
            .merge_state_from_json(&delta)
            .map_err(|err| EvalError::eval(err.to_string()))?;
        let batch_observable = observable
            .snapshot()
            .map_err(|err| EvalError::eval(err.to_string()))?;

        Ok(BatchResult::new(values, batch_observable))
    }
}

/// Test-only sampler-aggregator engine with simple random batch generation.
pub struct TestTrainingSamplerAggregator {
    batch_size: usize,
    continuous_dims: usize,
    discrete_dims: usize,
    training_target_samples: usize,
    training_delay_per_sample_ms: u64,
    trained_samples: usize,
    nr_batches: i64,
    nr_samples: i64,
    sum: f64,
}

impl TestTrainingSamplerAggregator {
    pub fn new(
        batch_size: usize,
        continuous_dims: usize,
        discrete_dims: usize,
        training_target_samples: usize,
        training_delay_per_sample_ms: u64,
    ) -> Self {
        Self {
            batch_size,
            continuous_dims,
            discrete_dims,
            training_target_samples,
            training_delay_per_sample_ms,
            trained_samples: 0,
            nr_batches: 0,
            nr_samples: 0,
            sum: 0.0,
        }
    }

    pub fn from_params(params: &JsonValue) -> Result<Self, BuildError> {
        let parsed: TestSamplerAggregatorParams = serde_json::from_value(params.clone())
            .map_err(|err| BuildError::build(format!("invalid sampler params: {err}")))?;
        Ok(Self::new(
            parsed.batch_size,
            parsed.continuous_dims,
            parsed.discrete_dims,
            parsed.training_target_samples,
            parsed.training_delay_per_sample_ms,
        ))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TestSamplerAggregatorParams {
    batch_size: usize,
    continuous_dims: usize,
    discrete_dims: usize,
    training_target_samples: usize,
    training_delay_per_sample_ms: u64,
}

impl Default for TestSamplerAggregatorParams {
    fn default() -> Self {
        Self {
            batch_size: 64,
            continuous_dims: 1,
            discrete_dims: 0,
            training_target_samples: 0,
            training_delay_per_sample_ms: 0,
        }
    }
}

impl SamplerAggregatorEngine for TestTrainingSamplerAggregator {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != self.continuous_dims {
            return Err(BuildError::build(format!(
                "test_only_training sampler expects continuous_dims={}, got {}",
                self.continuous_dims, point_spec.continuous_dims
            )));
        }
        if point_spec.discrete_dims != self.discrete_dims {
            return Err(BuildError::build(format!(
                "test_only_training sampler expects discrete_dims={}, got {}",
                self.discrete_dims, point_spec.discrete_dims
            )));
        }
        Ok(())
    }

    fn init(&mut self) -> Result<(), EngineError> {
        Ok(())
    }

    fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<Batch>, EngineError> {
        let mut rng = rand::thread_rng();
        let mut out = Vec::with_capacity(max_batches);

        for _ in 0..max_batches {
            let mut continuous_data = Vec::with_capacity(self.batch_size * self.continuous_dims);
            let mut discrete_data = Vec::with_capacity(self.batch_size * self.discrete_dims);
            for _ in 0..self.batch_size {
                continuous_data
                    .extend((0..self.continuous_dims).map(|_| rng.r#gen::<f64>() * 10.0));
                discrete_data.extend((0..self.discrete_dims).map(|_| rng.r#gen::<u32>() as i64));
            }
            let batch = Batch::from_flat_data(
                self.batch_size,
                self.continuous_dims,
                self.discrete_dims,
                continuous_data,
                discrete_data,
            )
            .map_err(|err| EngineError::engine(err.to_string()))?;
            out.push(batch);
        }

        Ok(out)
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        self.nr_batches += 1;
        self.nr_samples += training_weights.len() as i64;
        self.sum += training_weights.iter().sum::<f64>();

        if !training_weights.is_empty() && self.training_delay_per_sample_ms > 0 {
            let remaining_training = self
                .training_target_samples
                .saturating_sub(self.trained_samples);
            let delayed_samples = remaining_training.min(training_weights.len());
            if delayed_samples > 0 {
                thread::sleep(Duration::from_millis(
                    delayed_samples as u64 * self.training_delay_per_sample_ms,
                ));
            }
        }
        self.trained_samples = self.trained_samples.saturating_add(training_weights.len());

        let mean = if self.nr_samples > 0 {
            self.sum / self.nr_samples as f64
        } else {
            0.0
        };
        println!(
            "📈 test-only engine: batches={}, samples={}, mean={mean:.6}",
            self.nr_batches, self.nr_samples
        );

        Ok(())
    }
}
