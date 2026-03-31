use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::VecDeque;
use symbolica::numerical_integration::{Grid, Sample};
use tracing::info;

use crate::utils::domain::Domain;
use crate::{
    Batch, EngineError, LatentBatchSpec, Point, SamplePlan,
    core::BuildError,
    sampling::havana_grid::{build_havana_grid, sample_to_point, validate_havana_grid_domain},
    sampling::{LatentBatchPayload, SamplerAggregator, SamplerAggregatorSnapshot},
    utils::rng::SerializableMonteCarloRng,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaSamplerParams {
    pub seed: u64,
    pub bins: usize,
    pub min_samples_for_update: usize,
    pub samples_for_update: usize,
    pub initial_training_rate: f64,
    pub final_training_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HavanaInferenceSource {
    LatestTrainingSamplerAggregator,
    Snapshot { snapshot_id: i64 },
}

impl Default for HavanaInferenceSource {
    fn default() -> Self {
        Self::LatestTrainingSamplerAggregator
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaInferenceSamplerParams {
    pub seed: Option<u64>,
    pub source: HavanaInferenceSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HavanaSamplerSnapshot {
    batches_produced: usize,
    samples_produced: usize,
    batches_ingested: usize,
    samples_ingested: usize,
    samples_for_update: usize,
    stop_training_after_n_samples: usize,
    initial_training_rate: f64,
    final_training_rate: f64,
    grid: Grid<f64>,
    rng: SerializableMonteCarloRng,
    pending_training_samples: VecDeque<Vec<Sample<f64>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HavanaInferenceSamplerSnapshot {
    batches_produced: usize,
    samples_produced: usize,
    grid: Grid<f64>,
    rng: SerializableMonteCarloRng,
}

impl Default for HavanaSamplerParams {
    fn default() -> Self {
        Self {
            seed: 0,
            bins: 64,
            min_samples_for_update: 1_024,
            samples_for_update: 10_240,
            initial_training_rate: 0.1,
            final_training_rate: 0.1,
        }
    }
}

impl Default for HavanaInferenceSamplerParams {
    fn default() -> Self {
        Self {
            seed: None,
            source: HavanaInferenceSource::LatestTrainingSamplerAggregator,
        }
    }
}

fn validate_havana_sampler_params(
    parsed: &HavanaSamplerParams,
    _domain: &Domain,
) -> Result<(), BuildError> {
    if parsed.bins == 0 {
        return Err(BuildError::build("havana sampler requires bins > 0"));
    }
    if parsed.min_samples_for_update == 0 {
        return Err(BuildError::build(
            "havana sampler requires min_samples_for_update > 0",
        ));
    }
    if parsed.samples_for_update == 0 {
        return Err(BuildError::build(
            "havana sampler requires samples_for_update > 0",
        ));
    }
    if !parsed.initial_training_rate.is_finite() || parsed.initial_training_rate < 0.0 {
        return Err(BuildError::build(
            "havana sampler requires initial_training_rate >= 0",
        ));
    }
    if !parsed.final_training_rate.is_finite() || parsed.final_training_rate < 0.0 {
        return Err(BuildError::build(
            "havana sampler requires final_training_rate >= 0",
        ));
    }

    Ok(())
}

pub struct HavanaSampler {
    batches_produced: usize,
    samples_produced: usize,
    batches_ingested: usize,
    samples_ingested: usize,
    samples_for_update: usize,
    stop_training_after_n_samples: usize,
    initial_training_rate: f64,
    final_training_rate: f64,
    grid: Grid<f64>,
    rng: SerializableMonteCarloRng,
    pending_training_samples: VecDeque<Vec<Sample<f64>>>,
}

pub struct HavanaInferenceSampler {
    batches_produced: usize,
    samples_produced: usize,
    grid: Grid<f64>,
    rng: SerializableMonteCarloRng,
}

impl HavanaSampler {
    fn new(
        grid: Grid<f64>,
        rng: SerializableMonteCarloRng,
        samples_for_update: usize,
        stop_training_after_n_samples: usize,
        initial_training_rate: f64,
        final_training_rate: f64,
    ) -> Self {
        Self {
            batches_produced: 0,
            samples_produced: 0,
            batches_ingested: 0,
            samples_ingested: 0,
            samples_for_update,
            stop_training_after_n_samples,
            initial_training_rate,
            final_training_rate,
            grid,
            rng,
            pending_training_samples: VecDeque::new(),
        }
    }

    fn pending_training_sample_count(&self) -> usize {
        self.pending_training_samples.iter().map(Vec::len).sum()
    }

    fn training_window_samples_remaining(&self) -> usize {
        let remaining_training = self.remaining_training_samples_to_produce();
        if remaining_training == 0 {
            return 0;
        }

        let progressed_in_window = self.samples_ingested % self.samples_for_update;
        let remaining_in_window = if progressed_in_window == 0 {
            self.samples_for_update
        } else {
            self.samples_for_update - progressed_in_window
        };
        remaining_training.min(remaining_in_window)
    }

    fn remaining_training_samples_to_produce(&self) -> usize {
        self.stop_training_after_n_samples.saturating_sub(
            self.samples_ingested
                .saturating_add(self.pending_training_sample_count()),
        )
    }

    fn remaining_training_samples(&self) -> usize {
        self.stop_training_after_n_samples
            .saturating_sub(self.samples_ingested)
    }

    fn current_training_rate(&self) -> f64 {
        let progress = (self
            .samples_ingested
            .min(self.stop_training_after_n_samples) as f64)
            / (self.stop_training_after_n_samples as f64);
        if self.initial_training_rate <= 0.0 || self.final_training_rate <= 0.0 {
            return self.initial_training_rate
                + (self.final_training_rate - self.initial_training_rate) * progress;
        }

        self.initial_training_rate
            * (self.final_training_rate / self.initial_training_rate).powf(progress)
    }

    fn to_snapshot(&self) -> HavanaSamplerSnapshot {
        HavanaSamplerSnapshot {
            batches_produced: self.batches_produced,
            samples_produced: self.samples_produced,
            batches_ingested: self.batches_ingested,
            samples_ingested: self.samples_ingested,
            samples_for_update: self.samples_for_update,
            stop_training_after_n_samples: self.stop_training_after_n_samples,
            initial_training_rate: self.initial_training_rate,
            final_training_rate: self.final_training_rate,
            grid: self.grid.clone(),
            rng: self.rng.clone(),
            pending_training_samples: self.pending_training_samples.clone(),
        }
    }

    pub(crate) fn from_snapshot(
        snapshot: HavanaSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_havana_grid_domain(&snapshot.grid, domain, "havana snapshot")?;

        Ok(Self {
            batches_produced: snapshot.batches_produced,
            samples_produced: snapshot.samples_produced,
            batches_ingested: snapshot.batches_ingested,
            samples_ingested: snapshot.samples_ingested,
            samples_for_update: snapshot.samples_for_update,
            stop_training_after_n_samples: snapshot.stop_training_after_n_samples,
            initial_training_rate: snapshot.initial_training_rate,
            final_training_rate: snapshot.final_training_rate,
            grid: snapshot.grid,
            rng: snapshot.rng,
            pending_training_samples: snapshot.pending_training_samples,
        })
    }
}

impl HavanaSampler {
    pub(crate) fn from_params_and_domain(
        params: HavanaSamplerParams,
        domain: &Domain,
        stop_training_after_n_samples: usize,
    ) -> Result<Self, BuildError> {
        validate_havana_sampler_params(&params, domain)?;
        if stop_training_after_n_samples == 0 {
            return Err(BuildError::build(
                "havana sampler requires sample task nr_samples > 0",
            ));
        }

        let rng = SerializableMonteCarloRng::new(params.seed, 0);
        let grid = build_havana_grid(domain, &params)?;

        Ok(HavanaSampler::new(
            grid,
            rng,
            params.samples_for_update,
            stop_training_after_n_samples,
            params.initial_training_rate,
            params.final_training_rate,
        ))
    }

    pub(crate) fn into_inference(
        self,
        params: HavanaInferenceSamplerParams,
    ) -> HavanaInferenceSampler {
        HavanaInferenceSampler {
            batches_produced: 0,
            samples_produced: 0,
            grid: self.grid,
            rng: params
                .seed
                .map(|seed| SerializableMonteCarloRng::new(seed, 0))
                .unwrap_or(self.rng),
        }
    }
}

impl HavanaInferenceSampler {
    pub(crate) fn from_params_and_snapshot(
        params: HavanaInferenceSamplerParams,
        snapshot: SamplerAggregatorSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        match snapshot {
            SamplerAggregatorSnapshot::HavanaTraining { raw } => {
                let snapshot: HavanaSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana sampler snapshot for inference handoff: {err}"
                        ))
                    })?;
                let training = HavanaSampler::from_snapshot(snapshot, domain)?;
                Ok(training.into_inference(params))
            }
            SamplerAggregatorSnapshot::HavanaInference { raw } => {
                let snapshot: HavanaInferenceSamplerSnapshot = serde_json::from_value(raw)
                    .map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana inference sampler snapshot: {err}"
                        ))
                    })?;
                Self::from_snapshot(snapshot, domain)
            }
            SamplerAggregatorSnapshot::NaiveMonteCarlo { .. }
            | SamplerAggregatorSnapshot::RasterPlane { .. }
            | SamplerAggregatorSnapshot::RasterLine { .. } => Err(BuildError::build(
                "havana_inference sampler requires a havana snapshot for handoff",
            )),
        }
    }

    pub(crate) fn from_snapshot(
        snapshot: HavanaInferenceSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        validate_havana_grid_domain(&snapshot.grid, domain, "havana inference snapshot")?;
        Ok(Self {
            batches_produced: snapshot.batches_produced,
            samples_produced: snapshot.samples_produced,
            grid: snapshot.grid,
            rng: snapshot.rng,
        })
    }

    fn to_snapshot(&self) -> HavanaInferenceSamplerSnapshot {
        HavanaInferenceSamplerSnapshot {
            batches_produced: self.batches_produced,
            samples_produced: self.samples_produced,
            grid: self.grid.clone(),
            rng: self.rng.clone(),
        }
    }
}

impl SamplerAggregator for HavanaSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_havana_grid_domain(&self.grid, domain, "havana sampler")
    }

    fn training_samples_remaining(&self) -> Option<usize> {
        let remaining = self.remaining_training_samples_to_produce();
        if remaining == 0 {
            None
        } else {
            Some(remaining)
        }
    }

    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        if self.pending_training_sample_count() > 0 {
            return Ok(SamplePlan::Pause);
        }

        let nr_samples = self.training_window_samples_remaining();
        if nr_samples == 0 {
            Ok(SamplePlan::Pause)
        } else {
            Ok(SamplePlan::Produce { nr_samples })
        }
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        let raw = serde_json::to_value(self.to_snapshot()).map_err(|err| {
            EngineError::engine(format!("failed to serialize havana snapshot: {err}"))
        })?;
        Ok(SamplerAggregatorSnapshot::HavanaTraining { raw })
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        let mut points: Vec<Point> = Vec::with_capacity(nr_samples);

        if self.remaining_training_samples_to_produce() > 0 {
            let mut samples = Vec::with_capacity(nr_samples);
            for _ in 0..nr_samples {
                let mut sample = Sample::new();
                self.grid.sample(&mut self.rng, &mut sample);
                points.push(sample_to_point(&sample)?);
                samples.push(sample);
            }
            self.pending_training_samples.push_back(samples);
        } else {
            for _ in 0..nr_samples {
                let mut sample = Sample::new();
                self.grid.sample(&mut self.rng, &mut sample);
                points.push(sample_to_point(&sample)?);
            }
        }

        let batch = Batch::new(points).map_err(|err| EngineError::engine(err.to_string()))?;
        self.batches_produced += 1;
        self.samples_produced = self.samples_produced.saturating_add(nr_samples);
        Ok(LatentBatchSpec::from_batch(&batch))
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        let Some(samples) = self.pending_training_samples.pop_front() else {
            // Training is disabled for this batch or context is unavailable.
            return Ok(());
        };

        if training_weights.len() != samples.len() {
            return Err(EngineError::engine(format!(
                "training/sample size mismatch in Havana sampler: weights={}, samples={}",
                training_weights.len(),
                samples.len()
            )));
        }

        let before_samples_ingested = self.samples_ingested;
        let remaining_training = self.remaining_training_samples();
        let train_len = remaining_training.min(training_weights.len());
        for (eval, sample) in training_weights.iter().zip(samples.iter()).take(train_len) {
            self.grid
                .add_training_sample(sample, *eval / sample.get_weight()) // the evaluator return the weighted eval, so it needs to be divided by the sample weight
                .map_err(|err| EngineError::engine(err.to_string()))?;
        }
        self.batches_ingested += 1;
        self.samples_ingested = self.samples_ingested.saturating_add(train_len);

        if before_samples_ingested < self.stop_training_after_n_samples
            && self.samples_ingested >= self.stop_training_after_n_samples
        {
            info!(
                samples_ingested = self.samples_ingested,
                stop_training_after_n_samples = self.stop_training_after_n_samples,
                "havana sampler training complete"
            );
        }

        if train_len > 0 {
            let previous_bucket = before_samples_ingested / self.samples_for_update;
            let current_bucket = self.samples_ingested / self.samples_for_update;
            if current_bucket > previous_bucket {
                let training_rate = self.current_training_rate();
                for _ in 0..(current_bucket - previous_bucket) {
                    self.grid.update(training_rate, training_rate);
                }
            }
        }
        Ok(())
    }

    fn get_diagnostics(&mut self) -> serde_json::Value {
        let chi_sq = self.grid.get_statistics().chi_sq;
        json!({
            "chi_sq": chi_sq,
            "batches_produced": self.batches_produced,
            "samples_produced": self.samples_produced,
            "batches_ingested": self.batches_ingested,
            "samples_ingested": self.samples_ingested,
            "pending_training_batches": self.pending_training_samples.len(),
            "pending_training_samples": self.pending_training_sample_count(),
            "training_window_samples_remaining": self.training_window_samples_remaining(),
            "training_rate": self.current_training_rate(),
        })
    }
}

impl SamplerAggregator for HavanaInferenceSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_havana_grid_domain(&self.grid, domain, "havana inference sampler")
    }

    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError> {
        let seed = self.rng.next_u64();
        self.batches_produced += 1;
        self.samples_produced = self.samples_produced.saturating_add(nr_samples);
        Ok(LatentBatchSpec {
            nr_samples,
            observable: crate::core::ObservableConfig::Scalar,
            payload: LatentBatchPayload::HavanaInference { seed },
        })
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        if !training_weights.is_empty() {
            return Err(EngineError::engine(
                "havana inference sampler does not accept training weights",
            ));
        }
        Ok(())
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        let raw = serde_json::to_value(self.to_snapshot()).map_err(|err| {
            EngineError::engine(format!(
                "failed to serialize havana inference snapshot: {err}"
            ))
        })?;
        Ok(SamplerAggregatorSnapshot::HavanaInference { raw })
    }

    fn get_diagnostics(&mut self) -> serde_json::Value {
        json!({
            "batches_produced": self.batches_produced,
            "samples_produced": self.samples_produced,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    #[test]
    fn snapshot_roundtrip_restores_havana_runtime_state() {
        let domain = Domain::rectangular(2, 0);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            min_samples_for_update: 4,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 32)
            .expect("build havana sampler");
        let _ = sampler.produce_latent_batch(5).expect("produce");
        sampler
            .ingest_training_weights(&[1.0, 2.0, 3.0, 4.0, 5.0])
            .expect("ingest");
        let _ = sampler
            .produce_latent_batch(3)
            .expect("produce pending batch");

        let snapshot = sampler.snapshot().expect("snapshot");
        let restored = snapshot.into_runtime(&domain).expect("restore");
        let mut restored = restored;
        let restored_snapshot = restored.snapshot().expect("snapshot after restore");

        let SamplerAggregatorSnapshot::HavanaTraining { raw } = restored_snapshot else {
            panic!("expected havana snapshot");
        };
        let mut state: HavanaSamplerSnapshot =
            serde_json::from_value(raw).expect("decode restored havana snapshot");
        validate_havana_grid_domain(&state.grid, &domain, "havana snapshot")
            .expect("grid matches domain");
        assert_eq!(state.batches_produced, 2);
        assert_eq!(state.samples_produced, 8);
        assert_eq!(state.batches_ingested, 1);
        assert_eq!(state.samples_ingested, 5);
        assert_eq!(state.pending_training_samples.len(), 1);
        assert_eq!(state.rng.next_u64(), sampler.rng.next_u64());
    }

    #[test]
    fn havana_limits_training_production_by_pending_samples() {
        let domain = Domain::rectangular(2, 0);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            min_samples_for_update: 4,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 8)
            .expect("build havana sampler");

        assert_eq!(sampler.training_samples_remaining(), Some(8));
        let _ = sampler
            .produce_latent_batch(5)
            .expect("produce first training batch");
        assert_eq!(sampler.training_samples_remaining(), Some(3));
        let _ = sampler
            .produce_latent_batch(3)
            .expect("produce second training batch");
        assert_eq!(sampler.training_samples_remaining(), None);

        sampler
            .ingest_training_weights(&[1.0, 2.0, 3.0, 4.0, 5.0])
            .expect("ingest first batch");
        assert_eq!(sampler.training_samples_remaining(), None);

        sampler
            .ingest_training_weights(&[1.0, 2.0, 3.0])
            .expect("ingest second batch");
        assert_eq!(sampler.training_samples_remaining(), None);
    }

    #[test]
    fn havana_training_runs_in_lockstep_windows() {
        let domain = Domain::rectangular(2, 0);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            min_samples_for_update: 4,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 40)
            .expect("build havana sampler");

        assert_eq!(
            sampler.sample_plan().expect("initial sample plan"),
            SamplePlan::Produce { nr_samples: 16 }
        );

        let _ = sampler
            .produce_latent_batch(5)
            .expect("produce first batch");
        assert_eq!(
            sampler
                .sample_plan()
                .expect("pause while window is in flight"),
            SamplePlan::Pause
        );

        sampler
            .ingest_training_weights(&[1.0, 2.0, 3.0, 4.0, 5.0])
            .expect("ingest first batch");
        assert_eq!(
            sampler
                .sample_plan()
                .expect("emit remainder of the current training window"),
            SamplePlan::Produce { nr_samples: 11 }
        );

        let _ = sampler
            .produce_latent_batch(11)
            .expect("produce remainder of first window");
        assert_eq!(
            sampler
                .sample_plan()
                .expect("pause until second batch ingests"),
            SamplePlan::Pause
        );

        sampler
            .ingest_training_weights(&[1.0; 11])
            .expect("ingest remainder of first window");
        assert_eq!(
            sampler.sample_plan().expect("next full training window"),
            SamplePlan::Produce { nr_samples: 16 }
        );

        let _ = sampler
            .produce_latent_batch(16)
            .expect("produce second training window");
        sampler
            .ingest_training_weights(&[1.0; 16])
            .expect("ingest second training window");
        assert_eq!(
            sampler.sample_plan().expect("final partial window"),
            SamplePlan::Produce { nr_samples: 8 }
        );
    }

    #[test]
    fn havana_inference_handoff_emits_compact_seed_payloads() {
        let domain = Domain::rectangular(2, 0);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            min_samples_for_update: 4,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 8)
            .expect("build havana sampler");
        let _ = sampler
            .produce_latent_batch(4)
            .expect("produce training batch");
        sampler
            .ingest_training_weights(&[1.0, 2.0, 3.0, 4.0])
            .expect("ingest training batch");

        let snapshot = sampler.snapshot().expect("snapshot");
        let mut inference = HavanaInferenceSampler::from_params_and_snapshot(
            HavanaInferenceSamplerParams::default(),
            snapshot,
            &domain,
        )
        .expect("build inference sampler");
        let batch = inference
            .produce_latent_batch(5)
            .expect("produce inference");
        assert_eq!(batch.nr_samples, 5);
        match batch.payload {
            LatentBatchPayload::HavanaInference { .. } => {}
            other => panic!("expected havana_inference payload, got {other:?}"),
        }
        assert_eq!(inference.training_samples_remaining(), None);
    }

    #[test]
    fn havana_sampler_produces_discrete_points_for_nested_domains() {
        let domain = Domain::discrete(
            Some("group".to_string()),
            [
                crate::DomainBranch::new(0, Domain::continuous(2)),
                crate::DomainBranch::new(
                    1,
                    Domain::discrete(
                        Some("orientation".to_string()),
                        [
                            crate::DomainBranch::new(0, Domain::continuous(1)),
                            crate::DomainBranch::new(1, Domain::continuous(1)),
                        ],
                    ),
                ),
            ],
        );
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            min_samples_for_update: 4,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 8)
            .expect("build havana sampler");

        let batch = sampler.produce_latent_batch(16).expect("produce batch");
        let batch = batch.payload.into_batch().expect("batch payload");

        assert_eq!(batch.size(), 16);
        assert!(
            batch
                .points()
                .iter()
                .all(|point| !point.discrete.is_empty())
        );
        assert!(
            batch
                .points()
                .iter()
                .all(|point| point.discrete[0] == 0 || point.discrete[0] == 1)
        );
    }

    #[test]
    fn havana_inference_snapshot_restores_discrete_grid_topology() {
        let domain = Domain::rectangular(2, 1);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            min_samples_for_update: 4,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 8)
            .expect("build havana sampler");
        let _ = sampler
            .produce_latent_batch(4)
            .expect("produce training batch");
        sampler
            .ingest_training_weights(&[1.0, 2.0, 3.0, 4.0])
            .expect("ingest training batch");

        let snapshot = sampler.snapshot().expect("snapshot");
        let mut inference = HavanaInferenceSampler::from_params_and_snapshot(
            HavanaInferenceSamplerParams::default(),
            snapshot,
            &domain,
        )
        .expect("build inference sampler");

        let SamplerAggregatorSnapshot::HavanaInference { raw } =
            inference.snapshot().expect("inference snapshot")
        else {
            panic!("expected havana inference snapshot");
        };
        let restored: HavanaInferenceSamplerSnapshot =
            serde_json::from_value(raw).expect("decode inference snapshot");
        validate_havana_grid_domain(&restored.grid, &domain, "havana inference snapshot")
            .expect("grid matches domain");
    }
}
