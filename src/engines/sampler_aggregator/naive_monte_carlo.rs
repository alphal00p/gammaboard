use crate::core::{Batch, PointSpec};
use crate::engines::{BuildError, EngineError, SamplerAggregator};
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use std::{thread, time::Duration};

/// Test-only sampler-aggregator engine with simple random batch generation.
pub struct NaiveMonteCarloSamplerAggregator {
    continuous_dims: usize,
    discrete_dims: usize,
    training_target_samples: usize,
    training_delay_per_sample_ms: u64,
    trained_samples: usize,
    nr_batches: i64,
    nr_samples: i64,
    sum: f64,
}

impl NaiveMonteCarloSamplerAggregator {
    pub fn new(
        continuous_dims: usize,
        discrete_dims: usize,
        training_target_samples: usize,
        training_delay_per_sample_ms: u64,
    ) -> Self {
        Self {
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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NaiveMonteCarloSamplerParams {
    training_target_samples: usize,
    training_delay_per_sample_ms: u64,
}

impl Default for NaiveMonteCarloSamplerParams {
    fn default() -> Self {
        Self {
            training_target_samples: 0,
            training_delay_per_sample_ms: 0,
        }
    }
}

impl NaiveMonteCarloSamplerAggregator {
    pub(crate) fn from_params_and_point_spec(
        params: NaiveMonteCarloSamplerParams,
        point_spec: &PointSpec,
    ) -> Result<Self, BuildError> {
        Ok(Self::new(
            point_spec.continuous_dims,
            point_spec.discrete_dims,
            params.training_target_samples,
            params.training_delay_per_sample_ms,
        ))
    }
}

impl SamplerAggregator for NaiveMonteCarloSamplerAggregator {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != self.continuous_dims {
            return Err(BuildError::build(format!(
                "naive_monte_carlo sampler expects continuous_dims={}, got {}",
                self.continuous_dims, point_spec.continuous_dims
            )));
        }
        if point_spec.discrete_dims != self.discrete_dims {
            return Err(BuildError::build(format!(
                "naive_monte_carlo sampler expects discrete_dims={}, got {}",
                self.discrete_dims, point_spec.discrete_dims
            )));
        }
        Ok(())
    }

    fn is_training_active(&self) -> bool {
        self.training_target_samples == 0 || self.trained_samples < self.training_target_samples
    }

    fn get_max_samples(&self) -> Option<usize> {
        if self.training_target_samples == 0 {
            None
        } else {
            Some(
                self.training_target_samples
                    .saturating_sub(self.trained_samples),
            )
        }
    }

    fn export_checkpoint(&mut self) -> Result<JsonValue, EngineError> {
        Ok(json!({
            "kind": "naive_monte_carlo",
            "continuous_dims": self.continuous_dims,
            "discrete_dims": self.discrete_dims,
            "training_target_samples": self.training_target_samples,
            "training_delay_per_sample_ms": self.training_delay_per_sample_ms,
            "trained_samples": self.trained_samples,
            "nr_batches": self.nr_batches,
            "nr_samples": self.nr_samples,
            "sum": self.sum,
        }))
    }

    fn import_checkpoint(&mut self, checkpoint: &JsonValue) -> Result<(), EngineError> {
        let kind = checkpoint
            .get("kind")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        if kind != "naive_monte_carlo" {
            return Err(EngineError::engine(format!(
                "invalid naive_monte_carlo checkpoint kind: {kind}"
            )));
        }
        let trained_samples = checkpoint
            .get("trained_samples")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| {
                EngineError::engine("naive_monte_carlo checkpoint missing trained_samples")
            })?;
        let nr_batches = checkpoint
            .get("nr_batches")
            .and_then(JsonValue::as_i64)
            .ok_or_else(|| {
                EngineError::engine("naive_monte_carlo checkpoint missing nr_batches")
            })?;
        let nr_samples = checkpoint
            .get("nr_samples")
            .and_then(JsonValue::as_i64)
            .ok_or_else(|| {
                EngineError::engine("naive_monte_carlo checkpoint missing nr_samples")
            })?;
        let sum = checkpoint
            .get("sum")
            .and_then(JsonValue::as_f64)
            .ok_or_else(|| EngineError::engine("naive_monte_carlo checkpoint missing sum"))?;
        self.trained_samples = trained_samples as usize;
        self.nr_batches = nr_batches;
        self.nr_samples = nr_samples;
        self.sum = sum;
        Ok(())
    }

    fn produce_batch(&mut self, nr_samples: usize) -> Result<Batch, EngineError> {
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "naive_monte_carlo sampler requires nr_samples > 0",
            ));
        }
        let mut rng = rand::thread_rng();
        let mut continuous_data = Vec::with_capacity(nr_samples * self.continuous_dims);
        let mut discrete_data = Vec::with_capacity(nr_samples * self.discrete_dims);
        for _ in 0..nr_samples {
            continuous_data.extend((0..self.continuous_dims).map(|_| rng.r#gen::<f64>()));
            discrete_data.extend((0..self.discrete_dims).map(|_| rng.r#gen::<u32>() as i64));
        }

        let batch = Batch::from_flat_data(
            nr_samples,
            self.continuous_dims,
            self.discrete_dims,
            continuous_data,
            discrete_data,
        )
        .map_err(|err| EngineError::engine(err.to_string()))?;
        Ok(batch)
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        let accepted = if self.training_target_samples == 0 {
            training_weights.len()
        } else {
            self.training_target_samples
                .saturating_sub(self.trained_samples)
                .min(training_weights.len())
        };

        self.nr_batches += 1;
        self.nr_samples += accepted as i64;
        self.sum += training_weights.iter().take(accepted).sum::<f64>();

        if accepted > 0 && self.training_delay_per_sample_ms > 0 {
            if self.training_target_samples > 0 {
                thread::sleep(Duration::from_millis(
                    accepted as u64 * self.training_delay_per_sample_ms,
                ));
            }
        }
        self.trained_samples = self.trained_samples.saturating_add(accepted);
        Ok(())
    }
}
