use crate::core::{Batch, PointSpec};
use crate::engines::sampler_aggregator::BatchContext;
use crate::engines::{BuildError, EngineError, SamplerAggregator};
use rand::Rng;
use serde::Deserialize;
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

    fn produce_batch(
        &mut self,
        nr_samples: usize,
    ) -> Result<(Batch, Option<BatchContext>), EngineError> {
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
        Ok((batch, None))
    }

    fn ingest_training_weights(
        &mut self,
        training_weights: &[f64],
        _context: Option<BatchContext>,
    ) -> Result<(), EngineError> {
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
        Ok(())
    }
}
