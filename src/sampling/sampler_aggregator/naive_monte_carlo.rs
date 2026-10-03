use crate::core::{AccumulatorConfig, BuildError, EngineError};
use crate::sampling::latent_batch::IndexedBatchBuilder;
use crate::sampling::{
    DiscreteSubspace, Generation, LatentBatchSpec, PdfPoint, SamplerAggregator,
    SamplerAggregatorSnapshot,
};
use crate::utils::domain::Domain;
use crate::utils::synthetic_timing::{TimingModel, TimingStats};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// Synthetic uniform sampler with optional repeating training barriers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NaiveMonteCarloSamplerAggregator {
    domain: Domain,
    params: NaiveMonteCarloSamplerParams,
    rng: crate::utils::rng::SerializableMonteCarloRng,
    produced_samples: u64,
    returned_samples: u64,
    pending_training_samples: usize,
    window_returned: usize,
    updates: u64,
    produced_batches_total: usize,
    generation_stats: TimingStats,
    ingest_stats: TimingStats,
    update_stats: TimingStats,
    training_barrier_seconds: f64,
    #[serde(skip)]
    barrier_since: Option<std::time::Instant>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NaiveMonteCarloSamplerParams {
    pub seed: u64,
    /// Zero means inference; otherwise updates repeat after each complete window.
    pub training_window_samples: usize,
    pub generation_timing: TimingModel,
    pub ingest_timing: TimingModel,
    pub update_timing: TimingModel,
    pub fail_on_produce_batch_nr: Option<usize>,
    pub fail_on_materialize_batch_nr: Option<usize>,
}

impl NaiveMonteCarloSamplerAggregator {
    pub(crate) fn from_params_and_domain(
        params: NaiveMonteCarloSamplerParams,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        params.generation_timing.validate()?;
        params.ingest_timing.validate()?;
        params.update_timing.validate()?;
        Ok(Self {
            domain: domain.clone(),
            rng: crate::utils::rng::SerializableMonteCarloRng::new(params.seed, 0),
            params,
            produced_samples: 0,
            returned_samples: 0,
            pending_training_samples: 0,
            window_returned: 0,
            updates: 0,
            produced_batches_total: 0,
            generation_stats: TimingStats::default(),
            ingest_stats: TimingStats::default(),
            update_stats: TimingStats::default(),
            training_barrier_seconds: 0.0,
            barrier_since: None,
        })
    }

    pub(crate) fn from_snapshot(mut snapshot: Self, domain: &Domain) -> Result<Self, BuildError> {
        snapshot.validate_domain(domain)?;
        snapshot.params.generation_timing.validate()?;
        snapshot.params.ingest_timing.validate()?;
        snapshot.params.update_timing.validate()?;
        let window = snapshot.params.training_window_samples;
        if window > 0
            && (snapshot.window_returned >= window
                || snapshot.pending_training_samples > window - snapshot.window_returned)
        {
            return Err(BuildError::build(
                "invalid synthetic training window snapshot",
            ));
        }
        if snapshot.training_samples_remaining() == Some(0) {
            snapshot.barrier_since = Some(std::time::Instant::now());
        }
        Ok(snapshot)
    }

    fn training_samples_remaining(&self) -> Option<usize> {
        let window = self.params.training_window_samples;
        (window > 0).then(|| window - self.window_returned - self.pending_training_samples)
    }

    fn flush_barrier_time(&mut self) {
        if let Some(start) = self.barrier_since.take() {
            self.training_barrier_seconds += start.elapsed().as_secs_f64();
        }
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

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        if self.barrier_since.is_some() {
            self.flush_barrier_time();
            self.barrier_since = Some(std::time::Instant::now());
        }
        Ok(SamplerAggregatorSnapshot::NaiveMonteCarlo {
            raw: serde_json::to_value(&*self)?,
        })
    }

    fn generate(&mut self, max_samples: usize) -> Result<Generation, EngineError> {
        if max_samples == 0 {
            return Err(EngineError::invalid_input("max_samples must be positive"));
        }
        let training_remaining = self.training_samples_remaining();
        if training_remaining == Some(0) {
            return Ok(Generation::Waiting);
        }
        let nr_samples = max_samples.min(training_remaining.unwrap_or(usize::MAX));
        self.produced_batches_total += 1;
        if self
            .params
            .fail_on_produce_batch_nr
            .is_some_and(|n| n > 0 && self.produced_batches_total == n)
        {
            return Err(EngineError::engine(format!(
                "naive_monte_carlo injected produce failure on batch {}",
                self.produced_batches_total
            )));
        }
        self.params.generation_timing.wait(
            nr_samples,
            self.produced_samples ^ 0x67656e,
            &mut self.generation_stats,
        )?;
        let mut builder = IndexedBatchBuilder::new(nr_samples);
        let mut discrete = Vec::new();
        let mut continuous = Vec::new();
        for _ in 0..nr_samples {
            discrete.clear();
            continuous.clear();
            sample_domain_point(&self.domain, &mut self.rng, &mut discrete, &mut continuous)?;
            builder.push(&discrete, &continuous, 1.0);
        }
        self.produced_samples += nr_samples as u64;
        if self.params.training_window_samples > 0 {
            self.pending_training_samples += nr_samples;
            if self.training_samples_remaining() == Some(0) {
                self.barrier_since = Some(std::time::Instant::now());
            }
        }
        Ok(Generation::batch(
            LatentBatchSpec {
                nr_samples,
                accumulator: AccumulatorConfig::scalar(),
                payload: builder.finish(),
            },
            training_remaining,
        ))
    }

    fn feedback(&mut self, values: &[f64]) -> Result<(), EngineError> {
        if values.is_empty() {
            return Ok(());
        }
        if self.params.training_window_samples == 0 || values.len() > self.pending_training_samples
        {
            return Err(EngineError::engine("unexpected synthetic training values"));
        }
        self.params.ingest_timing.wait(
            values.len(),
            self.returned_samples ^ 0x696e67,
            &mut self.ingest_stats,
        )?;
        self.pending_training_samples -= values.len();
        self.window_returned += values.len();
        self.returned_samples += values.len() as u64;
        if self.window_returned == self.params.training_window_samples {
            self.params.update_timing.wait(
                self.window_returned,
                self.updates ^ 0x757064,
                &mut self.update_stats,
            )?;
            self.flush_barrier_time();
            self.updates += 1;
            self.window_returned = 0;
        }
        Ok(())
    }

    fn get_diagnostics(&mut self) -> serde_json::Value {
        serde_json::json!({"synthetic":true,"produced_samples":self.produced_samples,"returned_samples":self.returned_samples,
            "training_updates":self.updates,"training_barrier_seconds":self.training_barrier_seconds + self.barrier_since.map_or(0.0, |start| start.elapsed().as_secs_f64()),"training_window_samples":self.params.training_window_samples,
            "pending_training_samples":self.pending_training_samples,"window_returned":self.window_returned,
            "generation_timing":self.generation_stats,"ingest_timing":self.ingest_stats,"update_timing":self.update_stats})
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
    discrete: &mut Vec<i64>,
    continuous: &mut Vec<f64>,
) -> Result<(), EngineError> {
    match domain {
        Domain::Continuous { dims } => {
            continuous.extend((0..*dims).map(|_| rng.random::<f64>()));
        }
        Domain::Rectangular {
            discrete_cardinalities,
            continuous_dims,
        } => {
            if discrete_cardinalities.contains(&0) {
                return Err(EngineError::engine(
                    "naive_monte_carlo cannot sample rectangular domains with zero-cardinality discrete axes",
                ));
            }
            discrete.extend(
                discrete_cardinalities
                    .iter()
                    .map(|cardinality| rng.random_range(0..*cardinality) as i64),
            );
            continuous.extend((0..*continuous_dims).map(|_| rng.random::<f64>()));
        }
        Domain::Discrete { branches, .. } => {
            if branches.is_empty() {
                return Err(EngineError::engine(
                    "naive_monte_carlo cannot sample a discrete domain with no branches",
                ));
            }
            let branch = &branches[rng.random_range(0..branches.len())];
            discrete.push(branch.index as i64);
            sample_domain_point(&branch.domain, rng, discrete, continuous)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Batch, DomainBranch, Point};

    #[test]
    fn flat_generation_preserves_seeded_points_and_rng_across_domain_shapes() {
        // The original recursive, allocating algorithm is the compatibility reference.
        fn reference(domain: &Domain, rng: &mut impl Rng) -> (Vec<i64>, Vec<f64>) {
            match domain {
                Domain::Continuous { dims } => (vec![], (0..*dims).map(|_| rng.random()).collect()),
                Domain::Rectangular {
                    discrete_cardinalities,
                    continuous_dims,
                } => (
                    discrete_cardinalities
                        .iter()
                        .map(|n| rng.random_range(0..*n) as i64)
                        .collect(),
                    (0..*continuous_dims).map(|_| rng.random()).collect(),
                ),
                Domain::Discrete { branches, .. } => {
                    let branch = &branches[rng.random_range(0..branches.len())];
                    let (mut discrete, continuous) = reference(&branch.domain, rng);
                    discrete.insert(0, branch.index as i64);
                    (discrete, continuous)
                }
            }
        }
        for domain in [
            Domain::continuous(0),
            Domain::continuous(6),
            Domain::rectangular_with_cardinalities(2, [2, 3]),
            Domain::discrete(
                None,
                [
                    DomainBranch::new(7, Domain::continuous(0)),
                    DomainBranch::new(42, Domain::rectangular_with_cardinalities(3, [2, 3])),
                    DomainBranch::new(
                        9,
                        Domain::discrete(None, [DomainBranch::new(11, Domain::continuous(2))]),
                    ),
                ],
            ),
        ] {
            let mut sampler = NaiveMonteCarloSamplerAggregator::from_params_and_domain(
                NaiveMonteCarloSamplerParams {
                    seed: 42,
                    ..Default::default()
                },
                &domain,
            )
            .unwrap();
            let mut rng = sampler.rng.clone();
            for count in [3, 51, 74] {
                let expected = Batch::from_points((0..count).map(|_| {
                    let (discrete, continuous) = reference(&domain, &mut rng);
                    Point::new(continuous, discrete, 1.0)
                }))
                .unwrap();
                let actual = sampler
                    .generate(count)
                    .and_then(|generated| generated.into_batch())
                    .unwrap();
                assert_eq!(actual.payload.as_batch().unwrap(), expected);
                assert_eq!(actual, LatentBatchSpec::from_batch(&expected));
                assert_eq!(sampler.rng, rng);
            }
        }
    }

    fn sampler(window: usize) -> NaiveMonteCarloSamplerAggregator {
        NaiveMonteCarloSamplerAggregator::from_params_and_domain(
            NaiveMonteCarloSamplerParams {
                training_window_samples: window,
                seed: 42,
                ..Default::default()
            },
            &Domain::continuous(2),
        )
        .unwrap()
    }
    #[test]
    fn repeating_barrier_waits_for_all_returns_and_survives_restore() {
        let mut s = sampler(10);
        s.generate(6)
            .and_then(|generated| generated.into_batch())
            .unwrap();
        s.generate(4)
            .and_then(|generated| generated.into_batch())
            .unwrap();
        assert!(matches!(s.generate(10).unwrap(), Generation::Waiting));
        assert!(
            s.generate(1)
                .and_then(|generated| generated.into_batch())
                .is_err()
        );
        s.feedback(&[1.0; 4]).unwrap();
        assert_eq!(s.updates, 0);
        let SamplerAggregatorSnapshot::NaiveMonteCarlo { raw } = s.snapshot().unwrap() else {
            unreachable!()
        };
        let mut restored = NaiveMonteCarloSamplerAggregator::from_snapshot(
            serde_json::from_value(raw).unwrap(),
            &s.domain,
        )
        .unwrap();
        for runtime in [&mut s, &mut restored] {
            runtime.feedback(&[1.0; 6]).unwrap();
            assert_eq!(runtime.training_samples_remaining(), Some(10));
            assert_eq!(runtime.updates, 1);
        }
        assert_eq!(
            s.generate(10)
                .and_then(|generated| generated.into_batch())
                .unwrap(),
            restored
                .generate(10)
                .and_then(|generated| generated.into_batch())
                .unwrap()
        );
        restored.feedback(&[1.0; 10]).unwrap();
        assert_eq!(restored.updates, 2);
        assert_eq!(restored.update_stats.calls, 2);
        assert!(restored.feedback(&[1.0]).is_err());
    }
    #[test]
    fn barrier_checkpoint_preserves_elapsed_time_without_serializing_clock() {
        let mut s = sampler(10);
        s.generate(10)
            .and_then(|generated| generated.into_batch())
            .unwrap();
        s.training_barrier_seconds = 3.0;
        let SamplerAggregatorSnapshot::NaiveMonteCarlo { raw } = s.snapshot().unwrap() else {
            unreachable!()
        };
        assert!(raw.get("barrier_since").is_none());
        let saved = raw["training_barrier_seconds"].as_f64().unwrap();
        assert!(saved >= 3.0);
        let mut restored = NaiveMonteCarloSamplerAggregator::from_snapshot(
            serde_json::from_value(raw).unwrap(),
            &s.domain,
        )
        .unwrap();
        assert_eq!(restored.training_barrier_seconds, saved);
        assert!(restored.barrier_since.is_some());
        restored.feedback(&[1.0; 10]).unwrap();
        assert!(restored.barrier_since.is_none());
        assert!(restored.training_barrier_seconds >= saved);
    }

    #[test]
    fn inference_has_no_barrier_and_points_ignore_batch_partitioning() {
        let mut whole = sampler(0);
        let mut split = sampler(0);
        assert_eq!(whole.training_samples_remaining(), None);
        let a = whole
            .generate(10)
            .and_then(|generated| generated.into_batch())
            .unwrap();
        let b = split
            .generate(4)
            .and_then(|generated| generated.into_batch())
            .unwrap();
        let c = split
            .generate(6)
            .and_then(|generated| generated.into_batch())
            .unwrap();
        // RNG state after the same number of samples is independent of grouping.
        assert_eq!(whole.rng, split.rng);
        assert_eq!(a.nr_samples, b.nr_samples + c.nr_samples);
        assert_eq!(whole.update_stats.calls, 0);
    }
}
