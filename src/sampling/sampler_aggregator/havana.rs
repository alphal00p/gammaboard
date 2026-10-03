use crate::core::EngineResultExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::VecDeque;
use symbolica::numerical_integration::{Grid, Sample};
use tracing::info;

use crate::utils::domain::Domain;
use crate::{
    EngineError, Generation, LatentBatchSpec,
    core::{AccumulatorConfig, BuildError},
    sampling::havana_grid::{build_havana_grid, sample_components, validate_havana_grid_domain},
    sampling::latent_batch::IndexedBatchBuilder,
    sampling::{
        DiscreteSubspace, LatentBatchPayload, PdfPoint, SamplerAggregator,
        SamplerAggregatorSnapshot,
    },
    utils::rng::SerializableMonteCarloRng,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaSamplerParams {
    pub seed: u64,
    pub bins: usize,
    pub samples_for_update: usize,
    pub initial_training_rate: f64,
    pub final_training_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HavanaInferenceSource {
    #[default]
    LatestTrainingSamplerAggregator,
    Snapshot {
        snapshot_id: i64,
    },
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
    reusable_samples: Vec<Sample<f64>>,
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
            reusable_samples: Vec::new(),
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

        let inflight_or_ingested = self
            .samples_ingested
            .saturating_add(self.pending_training_sample_count());
        let completed_windows = self.samples_ingested / self.samples_for_update;
        let current_window_end =
            (completed_windows.saturating_add(1)).saturating_mul(self.samples_for_update);
        let remaining_in_window = current_window_end.saturating_sub(inflight_or_ingested);
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
        crate::activate_symbolica_oem_license()
            .map_err(|err| BuildError::build(err.to_string()))?;
        validate_havana_grid_domain(&snapshot.grid, domain, "havana snapshot")?;

        Ok(Self {
            reusable_samples: Vec::new(),
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
            _ => Err(BuildError::build(
                "havana_inference sampler requires a havana snapshot for handoff",
            )),
        }
    }

    pub(crate) fn from_snapshot(
        snapshot: HavanaInferenceSamplerSnapshot,
        domain: &Domain,
    ) -> Result<Self, BuildError> {
        crate::activate_symbolica_oem_license()
            .map_err(|err| BuildError::build(err.to_string()))?;
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

fn continuous_grid_pdf(
    grid: &symbolica::numerical_integration::ContinuousGrid<f64>,
    continuous: &[f64],
) -> Result<f64, EngineError> {
    if grid.continuous_dimensions.len() != continuous.len() {
        return Err(EngineError::engine(format!(
            "continuous point dimension mismatch: expected {}, got {}",
            grid.continuous_dimensions.len(),
            continuous.len()
        )));
    }

    let mut pdf = 1.0_f64;
    for (dimension, &sample) in grid.continuous_dimensions.iter().zip(continuous.iter()) {
        if !(0.0..=1.0).contains(&sample) {
            return Ok(0.0);
        }
        let partitioning = &dimension.partitioning;
        let bin_index = if sample >= 1.0 {
            partitioning.len().saturating_sub(2)
        } else {
            partitioning
                .binary_search_by(|value| value.partial_cmp(&sample).unwrap())
                .unwrap_or_else(|index| index)
                .saturating_sub(1)
        };
        let left = partitioning
            .get(bin_index)
            .copied()
            .ok_or_else(|| EngineError::engine("continuous grid partitioning is empty"))?;
        let right = partitioning.get(bin_index + 1).copied().ok_or_else(|| {
            EngineError::engine("continuous grid partitioning is missing the upper edge")
        })?;
        let width = right - left;
        if width <= 0.0 {
            return Err(EngineError::engine(format!(
                "continuous grid has non-positive bin width at index {bin_index}"
            )));
        }
        pdf *= 1.0 / (((partitioning.len() - 1) as f64) * width);
    }
    Ok(pdf)
}

fn grid_pdf(grid: &Grid<f64>, point: &PdfPoint) -> Result<Option<f64>, EngineError> {
    fn recurse(
        grid: &Grid<f64>,
        discrete: &[i64],
        continuous: &[f64],
        discrete_index: &mut usize,
    ) -> Result<f64, EngineError> {
        match grid {
            Grid::Continuous(grid) => continuous_grid_pdf(grid, continuous),
            Grid::Discrete(grid) => {
                let Some(&branch_idx) = discrete.get(*discrete_index) else {
                    return Err(EngineError::engine(
                        "discrete point dimension mismatch for havana pdf query",
                    ));
                };
                let branch_idx = usize::try_from(branch_idx).map_err(|_| {
                    EngineError::engine(format!(
                        "negative discrete index {branch_idx} in havana pdf query"
                    ))
                })?;
                let Some(bin) = grid.bins.get(branch_idx) else {
                    return Ok(0.0);
                };
                *discrete_index += 1;
                let child_pdf = match bin.sub_grid.as_ref() {
                    Some(sub_grid) => recurse(sub_grid, discrete, continuous, discrete_index)?,
                    None => {
                        if !continuous.is_empty() {
                            return Err(EngineError::engine(
                                "continuous coordinates provided for discrete-only havana grid",
                            ));
                        }
                        1.0
                    }
                };
                Ok(bin.pdf * child_pdf)
            }
            Grid::Uniform(discrete_bins, continuous_grid) => {
                if discrete.len() != discrete_bins.len() {
                    return Err(EngineError::engine(
                        "uniform-grid discrete dimension mismatch in havana pdf query",
                    ));
                }
                for (index, nr_bins) in discrete.iter().zip(discrete_bins.iter()) {
                    let index = usize::try_from(*index).map_err(|_| {
                        EngineError::engine(format!(
                            "negative discrete index {} in uniform-grid pdf query",
                            index
                        ))
                    })?;
                    if index >= *nr_bins {
                        return Ok(0.0);
                    }
                }
                let discrete_pdf = discrete_bins
                    .iter()
                    .fold(1.0, |acc, bins| acc / (*bins as f64));
                Ok(discrete_pdf * continuous_grid_pdf(continuous_grid, continuous)?)
            }
        }
    }

    let (discrete, continuous) = point;
    let mut discrete_index = 0_usize;
    let pdf = recurse(grid, discrete, continuous, &mut discrete_index)?;
    if discrete_index != discrete.len() {
        return Err(EngineError::engine(
            "extra discrete coordinates remained after havana pdf traversal",
        ));
    }
    Ok(Some(pdf))
}

fn grid_discrete_subspace_pdf(
    grid: &Grid<f64>,
    fixed_dims: &std::collections::BTreeMap<usize, i64>,
    depth: usize,
) -> Result<f64, EngineError> {
    match grid {
        Grid::Continuous(_) => {
            if fixed_dims.keys().any(|dim| *dim >= depth) {
                Ok(0.0)
            } else {
                Ok(1.0)
            }
        }
        Grid::Discrete(grid) => {
            let bin_probability = |index: usize| -> f64 {
                let pdf_sum = grid
                    .bins
                    .iter()
                    .map(|bin| bin.pdf)
                    .filter(|pdf| pdf.is_finite() && *pdf > 0.0)
                    .sum::<f64>();
                if pdf_sum > 0.0 {
                    grid.bins
                        .get(index)
                        .map(|bin| {
                            if bin.pdf.is_finite() && bin.pdf > 0.0 {
                                bin.pdf / pdf_sum
                            } else {
                                0.0
                            }
                        })
                        .unwrap_or(0.0)
                } else if grid.bins.is_empty() {
                    0.0
                } else {
                    1.0 / grid.bins.len() as f64
                }
            };
            if let Some(value) = fixed_dims.get(&depth) {
                let value = usize::try_from(*value).map_err(|_| {
                    EngineError::engine(format!(
                        "negative discrete index {value} in havana discrete_pdf query"
                    ))
                })?;
                let Some(bin) = grid.bins.get(value) else {
                    return Ok(0.0);
                };
                let pdf = bin_probability(value);
                let child_pdf = match bin.sub_grid.as_ref() {
                    Some(sub_grid) => grid_discrete_subspace_pdf(sub_grid, fixed_dims, depth + 1)?,
                    None => {
                        if fixed_dims.keys().any(|dim| *dim > depth) {
                            0.0
                        } else {
                            1.0
                        }
                    }
                };
                return Ok(pdf * child_pdf);
            }

            let mut pdf = 0.0;
            for (index, bin) in grid.bins.iter().enumerate() {
                let child_pdf = match bin.sub_grid.as_ref() {
                    Some(sub_grid) => grid_discrete_subspace_pdf(sub_grid, fixed_dims, depth + 1)?,
                    None => {
                        if fixed_dims.keys().any(|dim| *dim > depth) {
                            0.0
                        } else {
                            1.0
                        }
                    }
                };
                pdf += bin_probability(index) * child_pdf;
            }
            Ok(pdf)
        }
        Grid::Uniform(discrete_bins, _) => {
            let mut pdf = 1.0;
            for (axis, nr_bins) in discrete_bins.iter().enumerate() {
                if *nr_bins == 0 {
                    return Ok(0.0);
                }
                if let Some(value) = fixed_dims.get(&(depth + axis)) {
                    let value = usize::try_from(*value).map_err(|_| {
                        EngineError::engine(format!(
                            "negative discrete index {value} in havana uniform discrete_pdf query"
                        ))
                    })?;
                    if value >= *nr_bins {
                        return Ok(0.0);
                    }
                    pdf /= *nr_bins as f64;
                }
            }
            if fixed_dims
                .keys()
                .any(|dim| *dim >= depth + discrete_bins.len())
            {
                Ok(0.0)
            } else {
                Ok(pdf)
            }
        }
    }
}

impl SamplerAggregator for HavanaSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_havana_grid_domain(&self.grid, domain, "havana sampler")
    }

    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError> {
        let raw = serde_json::to_value(self.to_snapshot()).map_err(|err| {
            EngineError::engine(format!("failed to serialize havana snapshot: {err}"))
        })?;
        Ok(SamplerAggregatorSnapshot::HavanaTraining { raw })
    }

    fn generate(&mut self, max_samples: usize) -> Result<Generation, EngineError> {
        if max_samples == 0 {
            return Err(EngineError::invalid_input("max_samples must be positive"));
        }
        if self.samples_produced >= self.stop_training_after_n_samples {
            return Ok(Generation::Finished);
        }
        let remaining = self.training_window_samples_remaining();
        if remaining == 0 {
            return Ok(Generation::Waiting);
        }
        let nr_samples = remaining.min(max_samples);
        let mut samples = std::mem::take(&mut self.reusable_samples);
        samples.resize_with(nr_samples, Sample::new);
        let mut builder = IndexedBatchBuilder::new(nr_samples);
        let mut discrete = Vec::new();
        for sample in &mut samples {
            self.grid.sample(&mut self.rng, sample);
            let (continuous, weight) = sample_components(sample, &mut discrete)?;
            builder.push(&discrete, continuous, weight);
        }
        self.pending_training_samples.push_back(samples);
        self.batches_produced += 1;
        self.samples_produced += nr_samples;
        Ok(Generation::batch(
            LatentBatchSpec {
                nr_samples,
                accumulator: AccumulatorConfig::scalar(),
                payload: builder.finish(),
            },
            Some(remaining),
        ))
    }

    fn feedback(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        let Some(samples) = self.pending_training_samples.front() else {
            return Err(EngineError::engine(format!(
                "havana sampler received {} training weights with no pending training batch",
                training_values.len()
            )));
        };

        if training_values.len() != samples.len() {
            return Err(EngineError::engine(format!(
                "training/sample size mismatch in Havana sampler: weights={}, samples={}",
                training_values.len(),
                samples.len()
            )));
        }

        let before_samples_ingested = self.samples_ingested;
        let remaining_training = self.remaining_training_samples();
        let train_len = remaining_training.min(training_values.len());
        for (eval, sample) in training_values.iter().zip(samples.iter()).take(train_len) {
            self.grid
                .add_training_sample(sample, *eval / sample.get_weight()) // the evaluator return the weighted eval, so it needs to be divided by the sample weight
                .engine_err()?;
        }
        self.reusable_samples = self.pending_training_samples.pop_front().unwrap();
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

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        points
            .iter()
            .map(|point| grid_pdf(&self.grid, point))
            .collect()
    }

    fn discrete_pdf_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        subspaces
            .iter()
            .map(|subspace| {
                grid_discrete_subspace_pdf(&self.grid, &subspace.fixed_dims, 0).map(Some)
            })
            .collect()
    }
}

impl SamplerAggregator for HavanaInferenceSampler {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_havana_grid_domain(&self.grid, domain, "havana inference sampler")
    }

    fn generate(&mut self, max_samples: usize) -> Result<Generation, EngineError> {
        if max_samples == 0 {
            return Err(EngineError::invalid_input("max_samples must be positive"));
        }
        let nr_samples = max_samples;
        let mut rng_states = Vec::with_capacity(
            nr_samples.div_ceil(crate::sampling::latent_batch::RNG_CHECKPOINT_STRIDE),
        );
        let mut sample = Sample::new();
        for index in 0..nr_samples {
            if index % crate::sampling::latent_batch::RNG_CHECKPOINT_STRIDE == 0 {
                rng_states.push(self.rng.clone());
            }
            self.grid.sample(&mut self.rng, &mut sample);
        }
        self.batches_produced += 1;
        self.samples_produced += nr_samples;
        Ok(Generation::batch(
            LatentBatchSpec {
                nr_samples,
                accumulator: AccumulatorConfig::scalar(),
                payload: LatentBatchPayload::HavanaInferenceIndexed {
                    rng_states,
                    offset: 0,
                },
            },
            None,
        ))
    }

    fn feedback(&mut self, training_values: &[f64]) -> Result<(), EngineError> {
        if !training_values.is_empty() {
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

    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        points
            .iter()
            .map(|point| grid_pdf(&self.grid, point))
            .collect()
    }

    fn discrete_pdf_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        subspaces
            .iter()
            .map(|subspace| {
                grid_discrete_subspace_pdf(&self.grid, &subspace.fixed_dims, 0).map(Some)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AccumulatorConfig;
    use crate::evaluation::Materializer;
    use crate::sampling::havana_grid::sample_to_point;
    use crate::sampling::materializer::HavanaInferenceMaterializer;
    use crate::sampling::{DiscreteSubspace, LatentBatch, StageHandoffOwned};
    use crate::{Batch, Point};
    use rand::RngCore;
    use std::collections::BTreeMap;

    #[test]
    fn flat_generation_preserves_grid_samples_rng_and_pending_training() {
        for domain in [
            Domain::continuous(3),
            Domain::rectangular_with_cardinalities(2, [2, 3]),
            Domain::discrete(
                None,
                [
                    crate::DomainBranch::new(0, Domain::continuous(1)),
                    crate::DomainBranch::new(1, Domain::continuous(3)),
                ],
            ),
        ] {
            for training in [false, true] {
                let mut sampler = HavanaSampler::from_params_and_domain(
                    HavanaSamplerParams {
                        seed: 42,
                        samples_for_update: 256,
                        ..Default::default()
                    },
                    &domain,
                    256,
                )
                .unwrap();
                let mut grid = sampler.grid.clone();
                let mut rng = sampler.rng.clone();
                for _ in 0..2 {
                    let mut samples = Vec::new();
                    let expected = Batch::from_points((0..128).map(|_| {
                        let mut sample = Sample::new();
                        grid.sample(&mut rng, &mut sample);
                        let point = sample_to_point(&sample).unwrap();
                        samples.push(sample);
                        point
                    }))
                    .unwrap();
                    let actual = sampler
                        .generate(128)
                        .and_then(|generated| generated.into_batch())
                        .unwrap();
                    assert_eq!(actual.payload.as_batch().unwrap(), expected);
                    assert_eq!(sampler.rng, rng);
                    assert_eq!(
                        serde_json::to_value(sampler.pending_training_samples.back().unwrap())
                            .unwrap(),
                        serde_json::to_value(samples).unwrap(),
                    );
                    if !training {
                        sampler.feedback(&vec![1.0; 128]).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn snapshot_roundtrip_restores_havana_runtime_state() {
        let domain = Domain::rectangular(2, 0);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 32)
            .expect("build havana sampler");
        let _ = sampler
            .generate(5)
            .and_then(|generated| generated.into_batch())
            .expect("produce");
        sampler
            .feedback(&[1.0, 2.0, 3.0, 4.0, 5.0])
            .expect("ingest");
        let _ = sampler
            .generate(3)
            .and_then(|generated| generated.into_batch())
            .expect("produce pending batch");

        let snapshot = sampler.snapshot().expect("snapshot");
        let restored = snapshot
            .into_runtime(&domain, serde_json::json!({}))
            .expect("restore");
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
    fn havana_training_draws_are_independent_of_windows_and_reuse_storage() {
        let params = HavanaSamplerParams {
            samples_for_update: 16,
            ..Default::default()
        };
        let mut sampler =
            HavanaSampler::from_params_and_domain(params, &Domain::continuous(2), 40).unwrap();
        for window in [16, 16, 8] {
            let mut draws = Vec::new();
            let mut total = 0;
            while total < window {
                let Generation::Batch {
                    batch,
                    training_remaining,
                } = sampler.generate(5).unwrap()
                else {
                    panic!("expected draw")
                };
                assert_eq!(training_remaining, Some(window - total));
                assert!(batch.nr_samples <= 5);
                total += batch.nr_samples;
                draws.push(batch.nr_samples);
            }
            assert!(matches!(
                sampler.generate(5).unwrap(),
                Generation::Waiting | Generation::Finished
            ));
            for size in draws {
                let values: Vec<_> = (0..size).map(|i| 1.0 + i as f64).collect();
                assert!(sampler.feedback(&values[..size - 1]).is_err());
                sampler.feedback(&values).unwrap();
                assert_eq!(sampler.reusable_samples.len(), size);
            }
        }
        assert_eq!(sampler.samples_ingested, 40);
        assert!(matches!(sampler.generate(5).unwrap(), Generation::Finished));
        assert!(sampler.pending_training_samples.is_empty());
    }

    #[test]
    fn havana_inference_handoff_emits_compact_rng_payloads() {
        let domain = Domain::rectangular(2, 0);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 8)
            .expect("build havana sampler");
        let _ = sampler
            .generate(4)
            .and_then(|generated| generated.into_batch())
            .expect("produce training batch");
        sampler
            .feedback(&[1.0, 2.0, 3.0, 4.0])
            .expect("ingest training batch");

        let snapshot = sampler.snapshot().expect("snapshot");
        let mut inference = HavanaInferenceSampler::from_params_and_snapshot(
            HavanaInferenceSamplerParams::default(),
            snapshot,
            &domain,
        )
        .expect("build inference sampler");
        let batch = inference
            .generate(5)
            .and_then(|generated| generated.into_batch())
            .expect("produce inference");
        assert_eq!(batch.nr_samples, 5);
        match batch.payload {
            LatentBatchPayload::HavanaInferenceIndexed { .. } => {}
            other => panic!("expected havana_inference payload, got {other:?}"),
        }
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
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 16)
            .expect("build havana sampler");

        let batch = sampler
            .generate(16)
            .and_then(|generated| generated.into_batch())
            .expect("produce batch");
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
    fn havana_discrete_pdf_handles_ragged_subspaces() {
        let domain = Domain::discrete(
            Some("d0".to_string()),
            [
                crate::DomainBranch::new(0, Domain::continuous(3)),
                crate::DomainBranch::new(
                    1,
                    Domain::discrete(
                        Some("d1".to_string()),
                        [
                            crate::DomainBranch::new(0, Domain::continuous(1)),
                            crate::DomainBranch::new(
                                1,
                                Domain::rectangular_with_cardinalities(5, [5]),
                            ),
                        ],
                    ),
                ),
            ],
        );
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 16)
            .expect("build havana sampler");

        let values = sampler
            .discrete_pdf_batch(&[
                DiscreteSubspace {
                    fixed_dims: BTreeMap::from([(0, 0)]),
                },
                DiscreteSubspace {
                    fixed_dims: BTreeMap::from([(0, 1)]),
                },
                DiscreteSubspace {
                    fixed_dims: BTreeMap::from([(0, 1), (1, 1), (2, 3)]),
                },
                DiscreteSubspace {
                    fixed_dims: BTreeMap::from([(0, 0), (1, 0)]),
                },
            ])
            .expect("discrete pdf");

        assert_eq!(values.len(), 4);
        assert!((values[0].unwrap() - 0.5).abs() < 1e-12);
        assert!((values[1].unwrap() - 0.5).abs() < 1e-12);
        assert!((values[2].unwrap() - 0.05).abs() < 1e-12);
        assert_eq!(values[3], Some(0.0));
    }

    #[test]
    fn havana_discrete_pdf_falls_back_to_uniform_for_all_zero_grid_probabilities() {
        let domain = Domain::discrete(
            Some("d0".to_string()),
            [
                crate::DomainBranch::new(0, Domain::continuous(1)),
                crate::DomainBranch::new(1, Domain::continuous(1)),
            ],
        );
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 16)
            .expect("build havana sampler");
        let Grid::Discrete(grid) = &mut sampler.grid else {
            panic!("expected discrete grid");
        };
        for bin in &mut grid.bins {
            bin.pdf = 0.0;
        }

        let values = sampler
            .discrete_pdf_batch(&[
                DiscreteSubspace {
                    fixed_dims: BTreeMap::from([(0, 0)]),
                },
                DiscreteSubspace {
                    fixed_dims: BTreeMap::from([(0, 1)]),
                },
            ])
            .expect("discrete pdf");

        assert_eq!(values, vec![Some(0.5), Some(0.5)]);
    }

    #[test]
    fn havana_inference_snapshot_restores_discrete_grid_topology() {
        let domain = Domain::rectangular(2, 1);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut sampler = HavanaSampler::from_params_and_domain(params, &domain, 8)
            .expect("build havana sampler");
        let _ = sampler
            .generate(4)
            .and_then(|generated| generated.into_batch())
            .expect("produce training batch");
        sampler
            .feedback(&[1.0, 2.0, 3.0, 4.0])
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

    #[test]
    fn havana_inference_is_expected_to_be_batch_partition_invariant() {
        for domain in [
            Domain::continuous(6),
            Domain::discrete(
                None,
                [
                    crate::DomainBranch::new(0, Domain::continuous(1)),
                    crate::DomainBranch::new(1, Domain::continuous(3)),
                ],
            ),
        ] {
            let params = HavanaSamplerParams {
                seed: 7,
                bins: 8,
                samples_for_update: 16,
                initial_training_rate: 0.1,
                final_training_rate: 0.01,
            };

            let mut training = HavanaSampler::from_params_and_domain(params, &domain, 32)
                .expect("build havana sampler");
            let _ = training
                .generate(16)
                .and_then(|generated| generated.into_batch())
                .expect("produce training batch");
            training
                .feedback(&[1.0; 16])
                .expect("ingest training weights");

            let training_snapshot = training.snapshot().expect("training snapshot");

            let collect_points = |batch_plan: &[usize]| -> Vec<Point> {
                let mut inference = HavanaInferenceSampler::from_params_and_snapshot(
                    HavanaInferenceSamplerParams::default(),
                    training_snapshot.clone(),
                    &domain,
                )
                .expect("build inference sampler");

                let mut points = Vec::new();
                for &nr_samples in batch_plan {
                    let latent = inference
                        .generate(nr_samples)
                        .and_then(|generated| generated.into_batch())
                        .expect("produce inference batch");
                    let handoff = StageHandoffOwned {
                        sampler_snapshot: Some(inference.snapshot().expect("inference snapshot")),
                        observable_state: None,
                    };
                    let mut materializer = HavanaInferenceMaterializer::new(Some(handoff.as_ref()))
                        .expect("build inference materializer");
                    let batch = materializer
                        .materialize_batch(&LatentBatch {
                            nr_samples: latent.nr_samples,
                            accumulator: AccumulatorConfig::scalar(),
                            payload: latent.payload,
                        })
                        .expect("materialize inference batch");
                    points.extend(batch.points().iter().cloned());
                }
                points
            };

            let one_batch = collect_points(&[4099]);
            let two_batches = collect_points(&[1023, 3, 2048, 1025]);
            let mut bulk = HavanaInferenceSampler::from_params_and_snapshot(
                HavanaInferenceSamplerParams::default(),
                training_snapshot,
                &domain,
            )
            .unwrap();
            let batch = bulk.generate(4099).unwrap().into_batch().unwrap().build();
            let handoff = StageHandoffOwned {
                sampler_snapshot: Some(bulk.snapshot().unwrap()),
                observable_state: None,
            };
            let mut materializer =
                HavanaInferenceMaterializer::new(Some(handoff.as_ref())).unwrap();
            let batch = LatentBatch::from_bytes(&batch.to_bytes().unwrap()).unwrap();
            let bulk_points: Vec<_> = (0..4099)
                .step_by(333)
                .flat_map(|start| {
                    materializer
                        .materialize_batch(&batch.slice(start, 333.min(4099 - start)).unwrap())
                        .unwrap()
                        .points()
                        .to_vec()
                })
                .collect();
            assert_eq!(
                bulk_points, one_batch,
                "bulk splitting must preserve the RNG stream"
            );
            assert_eq!(
                one_batch, two_batches,
                "havana inference should be independent of batch partitioning"
            );
        }
    }
}
