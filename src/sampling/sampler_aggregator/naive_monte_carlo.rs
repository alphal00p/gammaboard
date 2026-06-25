use crate::core::EngineResultExt;
use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Point};
use crate::sampling::{
    DiscreteSubspace, LatentBatchSpec, PdfPoint, SamplePlan, SamplerAggregator,
    SamplerAggregatorSnapshot,
};
use crate::utils::domain::Domain;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{thread, time::Duration};

/// Test-only sampler-aggregator engine with simple random batch generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NaiveMonteCarloSamplerAggregator {
    domain: Domain,
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
        domain: Domain,
        training_target_samples: usize,
        training_delay_per_sample_ms: u64,
        fail_on_produce_batch_nr: Option<usize>,
    ) -> Self {
        Self {
            domain,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NaiveMonteCarloSamplerParams {
    pub training_target_samples: usize,
    pub training_delay_per_sample_ms: u64,
    #[serde(default)]
    pub fail_on_produce_batch_nr: Option<usize>,
    #[serde(default)]
    pub fail_on_materialize_batch_nr: Option<usize>,
}

impl NaiveMonteCarloSamplerAggregator {
    pub(crate) fn from_params_and_domain(
        params: NaiveMonteCarloSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        Ok(Self::new(
            domain.clone(),
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
        if domain != &self.domain {
            return Err(BuildError::build(format!(
                "naive_monte_carlo sampler domain mismatch: expected {:?}, got {:?}",
                self.domain, domain
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
            let (discrete, continuous) = sample_domain_point(&self.domain, &mut rng)?;
            points.push(Point::new(continuous, discrete, 1.0));
        }

        let batch = Batch::new(points).engine_err()?;
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

    fn ingest_training_values(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        let accepted = if self.training_target_samples == 0 {
            training_values.len()
        } else {
            self.training_target_samples
                .saturating_sub(self.trained_samples)
                .min(training_values.len())
        };

        self.nr_batches += 1;
        self.nr_samples += accepted as i64;
        self.sum += training_values.iter().take(accepted).sum::<f64>();

        if accepted > 0 && self.training_delay_per_sample_ms > 0 && self.training_target_samples > 0
        {
            thread::sleep(Duration::from_millis(
                accepted as u64 * self.training_delay_per_sample_ms,
            ));
        }
        self.trained_samples = self.trained_samples.saturating_add(accepted);
        self.pending_training_samples = self
            .pending_training_samples
            .saturating_sub(training_values.len());
        Ok(())
    }

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        if let Some((continuous_dims, 0)) = self.domain.fixed_rectangular_dims() {
            return Ok(points
                .iter()
                .map(|(discrete, continuous)| {
                    if !discrete.is_empty() || continuous.len() != continuous_dims {
                        return None;
                    }
                    if continuous.iter().all(|value| (0.0..=1.0).contains(value)) {
                        Some(1.0)
                    } else {
                        Some(0.0)
                    }
                })
                .collect());
        }
        Ok(vec![None; points.len()])
    }

    fn discrete_pdf_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        subspaces
            .iter()
            .map(|subspace| {
                discrete_subspace_probability(&self.domain, &subspace.fixed_dims, 0).map(Some)
            })
            .collect()
    }
}

fn discrete_subspace_probability(
    domain: &Domain,
    fixed_dims: &std::collections::BTreeMap<usize, i64>,
    depth: usize,
) -> Result<f64, EngineError> {
    match domain {
        Domain::Continuous { .. } => {
            if fixed_dims.keys().any(|dim| *dim >= depth) {
                Ok(0.0)
            } else {
                Ok(1.0)
            }
        }
        Domain::Rectangular {
            discrete_cardinalities,
            ..
        } => {
            let mut probability = 1.0;
            for (axis, cardinality) in discrete_cardinalities.iter().enumerate() {
                if *cardinality == 0 {
                    return Ok(0.0);
                }
                if let Some(value) = fixed_dims.get(&(depth + axis)) {
                    let Ok(value) = usize::try_from(*value) else {
                        return Ok(0.0);
                    };
                    if value >= *cardinality {
                        return Ok(0.0);
                    }
                    probability /= *cardinality as f64;
                }
            }
            if fixed_dims
                .keys()
                .any(|dim| *dim >= depth + discrete_cardinalities.len())
            {
                Ok(0.0)
            } else {
                Ok(probability)
            }
        }
        Domain::Discrete { branches, .. } => {
            if branches.is_empty() {
                return Ok(0.0);
            }
            if let Some(value) = fixed_dims.get(&depth) {
                let Ok(value) = usize::try_from(*value) else {
                    return Ok(0.0);
                };
                let Some(branch) = branches.iter().find(|branch| branch.index == value) else {
                    return Ok(0.0);
                };
                return Ok(
                    discrete_subspace_probability(&branch.domain, fixed_dims, depth + 1)?
                        / branches.len() as f64,
                );
            }
            let mut probability = 0.0;
            for branch in branches {
                probability +=
                    discrete_subspace_probability(&branch.domain, fixed_dims, depth + 1)?
                        / branches.len() as f64;
            }
            Ok(probability)
        }
    }
}

fn sample_domain_point(
    domain: &Domain,
    rng: &mut impl Rng,
) -> Result<(Vec<i64>, Vec<f64>), EngineError> {
    match domain {
        Domain::Continuous { dims } => Ok((
            Vec::new(),
            (0..*dims).map(|_| rng.random::<f64>()).collect(),
        )),
        Domain::Rectangular {
            discrete_cardinalities,
            continuous_dims,
        } => {
            if discrete_cardinalities.contains(&0) {
                return Err(EngineError::engine(
                    "naive_monte_carlo cannot sample rectangular domains with zero-cardinality discrete axes",
                ));
            }
            Ok((
                discrete_cardinalities
                    .iter()
                    .map(|cardinality| rng.random_range(0..*cardinality) as i64)
                    .collect(),
                (0..*continuous_dims).map(|_| rng.random::<f64>()).collect(),
            ))
        }
        Domain::Discrete { branches, .. } => {
            if branches.is_empty() {
                return Err(EngineError::engine(
                    "naive_monte_carlo cannot sample a discrete domain with no branches",
                ));
            }
            let branch = &branches[rng.random_range(0..branches.len())];
            let (mut discrete, continuous) = sample_domain_point(&branch.domain, rng)?;
            discrete.insert(0, branch.index as i64);
            Ok((discrete, continuous))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_roundtrip_restores_naive_runtime_state() {
        let domain = Domain::rectangular(2, 1);
        let mut sampler = NaiveMonteCarloSamplerAggregator::new(domain.clone(), 100, 7, None);
        sampler.trained_samples = 13;
        sampler.nr_batches = 5;
        sampler.nr_samples = 29;
        sampler.sum = 4.5;

        let snapshot = sampler.snapshot().expect("snapshot");
        let mut restored = snapshot
            .into_runtime(&domain, serde_json::json!({}))
            .expect("restore");
        let restored_snapshot = restored.snapshot().expect("snapshot after restore");

        let SamplerAggregatorSnapshot::NaiveMonteCarlo { raw } = restored_snapshot else {
            panic!("expected naive snapshot");
        };
        let state: NaiveMonteCarloSamplerAggregator =
            serde_json::from_value(raw).expect("decode snapshot");
        assert_eq!(state.domain, domain);
        assert_eq!(state.training_target_samples, 100);
        assert_eq!(state.training_delay_per_sample_ms, 7);
        assert_eq!(state.trained_samples, 13);
        assert_eq!(state.nr_batches, 5);
        assert_eq!(state.nr_samples, 29);
        assert_eq!(state.sum, 4.5);
    }
}
