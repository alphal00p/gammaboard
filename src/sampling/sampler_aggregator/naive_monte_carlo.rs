use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Point};
use crate::sampling::{LatentBatchSpec, SamplePlan, SamplerAggregator, SamplerAggregatorSnapshot};
use crate::utils::domain::Domain;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{thread, time::Duration};

/// Test-only sampler-aggregator engine with simple random batch generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NaiveMonteCarloSamplerAggregator {
    continuous_dims: usize,
    discrete_dims: usize,
    training_target_samples: usize,
    training_delay_per_sample_ms: u64,
    trained_samples: usize,
    pending_training_samples: usize,
    fail_on_produce_batch_nr: Option<usize>,
    #[serde(default)]
    produced_batches_total: usize,
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
        fail_on_produce_batch_nr: Option<usize>,
    ) -> Self {
        Self {
            continuous_dims,
            discrete_dims,
            training_target_samples,
            training_delay_per_sample_ms,
            trained_samples: 0,
            pending_training_samples: 0,
            fail_on_produce_batch_nr,
            produced_batches_total: 0,
            nr_batches: 0,
            nr_samples: 0,
            sum: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct NaiveMonteCarloSamplerParams {
    pub training_target_samples: usize,
    pub training_delay_per_sample_ms: u64,
    #[serde(default)]
    pub fail_on_produce_batch_nr: Option<usize>,
    #[serde(default)]
    pub fail_on_materialize_batch_nr: Option<usize>,
}

impl Default for NaiveMonteCarloSamplerParams {
    fn default() -> Self {
        Self {
            training_target_samples: 0,
            training_delay_per_sample_ms: 0,
            fail_on_produce_batch_nr: None,
            fail_on_materialize_batch_nr: None,
        }
    }
}

impl NaiveMonteCarloSamplerAggregator {
    pub(crate) fn from_params_and_domain(
        params: NaiveMonteCarloSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        let (continuous_dims, discrete_dims) =
            domain.fixed_rectangular_dims().ok_or_else(|| {
                BuildError::build("naive_monte_carlo sampler requires a fixed rectangular domain")
            })?;
        Ok(Self::new(
            continuous_dims,
            discrete_dims,
            params.training_target_samples,
            params.training_delay_per_sample_ms,
            params.fail_on_produce_batch_nr,
        ))
    }

    pub(crate) fn from_snapshot(snapshot: Self, domain: &Domain) -> Result<Self, BuildError> {
        let runtime = snapshot;
        runtime.validate_domain(domain)?;
        Ok(runtime)
    }
}

impl SamplerAggregator for NaiveMonteCarloSamplerAggregator {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        let (continuous_dims, discrete_dims) =
            domain.fixed_rectangular_dims().ok_or_else(|| {
                BuildError::build("naive_monte_carlo sampler requires a fixed rectangular domain")
            })?;
        if continuous_dims != self.continuous_dims {
            return Err(BuildError::build(format!(
                "naive_monte_carlo sampler expects continuous_dims={}, got {}",
                self.continuous_dims, continuous_dims
            )));
        }
        if discrete_dims != self.discrete_dims {
            return Err(BuildError::build(format!(
                "naive_monte_carlo sampler expects discrete_dims={}, got {}",
                self.discrete_dims, discrete_dims
            )));
        }
        Ok(())
    }

    fn training_samples_remaining(&self) -> Option<usize> {
        if self.training_target_samples == 0 {
            None
        } else {
            Some(
                self.training_target_samples.saturating_sub(
                    self.trained_samples
                        .saturating_add(self.pending_training_samples),
                ),
            )
        }
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        Ok(SamplePlan::Produce {
            nr_samples: usize::MAX,
        })
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        Ok(SamplerAggregatorSnapshot::NaiveMonteCarlo {
            raw: serde_json::to_value(self.clone()).map_err(EngineError::from)?,
        })
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        self.produced_batches_total = self.produced_batches_total.saturating_add(1);
        if self
            .fail_on_produce_batch_nr
            .is_some_and(|n| n > 0 && self.produced_batches_total == n)
        {
            return Err(EngineError::engine(format!(
                "naive_monte_carlo injected produce failure on batch {}",
                self.produced_batches_total
            )));
        }
        if nr_samples == 0 {
            return Err(EngineError::engine(
                "naive_monte_carlo sampler requires nr_samples > 0",
            ));
        }
        let mut rng = rand::rng();
        let mut points = Vec::with_capacity(nr_samples);
        for _ in 0..nr_samples {
            points.push(Point::new(
                (0..self.continuous_dims)
                    .map(|_| rng.random::<f64>())
                    .collect(),
                (0..self.discrete_dims)
                    .map(|_| rng.random::<u32>() as i64)
                    .collect(),
                1.0,
            ));
        }

        let batch = Batch::new(points).map_err(|err| EngineError::engine(err.to_string()))?;
        if self.training_target_samples > 0 {
            let reserved = self
                .training_target_samples
                .saturating_sub(
                    self.trained_samples
                        .saturating_add(self.pending_training_samples),
                )
                .min(nr_samples);
            self.pending_training_samples = self.pending_training_samples.saturating_add(reserved);
        }
        Ok(LatentBatchSpec::from_batch(&batch))
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
        self.pending_training_samples = self
            .pending_training_samples
            .saturating_sub(training_weights.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_roundtrip_restores_naive_runtime_state() {
        let domain = Domain::rectangular(2, 1);
        let mut sampler = NaiveMonteCarloSamplerAggregator::new(2, 1, 100, 7, None);
        sampler.trained_samples = 13;
        sampler.nr_batches = 5;
        sampler.nr_samples = 29;
        sampler.sum = 4.5;

        let snapshot = sampler.snapshot().expect("snapshot");
        let mut restored = snapshot.into_runtime(&domain).expect("restore");
        let restored_snapshot = restored.snapshot().expect("snapshot after restore");

        let SamplerAggregatorSnapshot::NaiveMonteCarlo { raw } = restored_snapshot else {
            panic!("expected naive snapshot");
        };
        let state: NaiveMonteCarloSamplerAggregator =
            serde_json::from_value(raw).expect("decode snapshot");
        assert_eq!(state.continuous_dims, 2);
        assert_eq!(state.discrete_dims, 1);
        assert_eq!(state.training_target_samples, 100);
        assert_eq!(state.training_delay_per_sample_ms, 7);
        assert_eq!(state.trained_samples, 13);
        assert_eq!(state.nr_batches, 5);
        assert_eq!(state.nr_samples, 29);
        assert_eq!(state.sum, 4.5);
    }
}
