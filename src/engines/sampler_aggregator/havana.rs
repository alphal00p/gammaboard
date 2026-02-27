use serde::Deserialize;
use serde_json::json;
use symbolica::numerical_integration::{ContinuousGrid, Grid, MonteCarloRng, Sample};
use tracing::info;

use crate::{
    Batch, EngineError,
    batch::PointSpec,
    engines::{BatchContext, BuildError, BuildFromJson, SamplerAggregator},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaSamplerParams {
    seed: u64,
    continuous_dims: usize,
    bins: usize,
    min_samples_for_update: usize,
    samples_for_update: usize,
    stop_training_after_n_samples: Option<usize>,
    initial_training_rate: f64,
    final_training_rate: f64,
}

impl Default for HavanaSamplerParams {
    fn default() -> Self {
        Self {
            seed: 0,
            continuous_dims: 1,
            bins: 64,
            min_samples_for_update: 1_024,
            samples_for_update: 10_240,
            stop_training_after_n_samples: None,
            initial_training_rate: 0.1,
            final_training_rate: 0.1,
        }
    }
}

fn validate_havana_sampler_params(parsed: &HavanaSamplerParams) -> Result<(), BuildError> {
    if parsed.continuous_dims == 0 {
        return Err(BuildError::build(
            "havana sampler requires continuous_dims > 0",
        ));
    }
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
    if parsed.stop_training_after_n_samples.is_none() {
        return Err(BuildError::build(
            "havana sampler requires stop_training_after_n_samples",
        ));
    }
    if parsed.stop_training_after_n_samples == Some(0) {
        return Err(BuildError::build(
            "havana sampler stop_training_after_n_samples must be > 0",
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
    continuous_dims: usize,
    batches_produced: usize,
    samples_produced: usize,
    batches_ingested: usize,
    samples_ingested: usize,
    samples_for_update: usize,
    stop_training_after_n_samples: usize,
    initial_training_rate: f64,
    final_training_rate: f64,
    grid: Grid<f64>,
    rng: MonteCarloRng,
}

impl HavanaSampler {
    fn new(
        continuous_dims: usize,
        grid: Grid<f64>,
        rng: MonteCarloRng,
        samples_for_update: usize,
        stop_training_after_n_samples: usize,
        initial_training_rate: f64,
        final_training_rate: f64,
    ) -> Self {
        Self {
            continuous_dims,
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
        }
    }

    fn remaining_training_samples(&self) -> usize {
        self.stop_training_after_n_samples
            .saturating_sub(self.samples_ingested)
    }

    fn is_training_active(&self) -> bool {
        self.samples_ingested < self.stop_training_after_n_samples
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
}

impl BuildFromJson for HavanaSampler {
    type Params = HavanaSamplerParams;
    fn from_parsed_params(params: Self::Params) -> Result<Self, BuildError> {
        validate_havana_sampler_params(&params)?;

        let rng = MonteCarloRng::new(params.seed, 0);
        let grid = Grid::Continuous(ContinuousGrid::new(
            params.continuous_dims,
            params.bins,
            params.min_samples_for_update,
            None,
            false,
        ));
        let stop_training_after_n_samples =
            params.stop_training_after_n_samples.ok_or_else(|| {
                BuildError::build("havana sampler requires stop_training_after_n_samples")
            })?;

        Ok(HavanaSampler::new(
            params.continuous_dims,
            grid,
            rng,
            params.samples_for_update,
            stop_training_after_n_samples,
            params.initial_training_rate,
            params.final_training_rate,
        ))
    }
}

impl SamplerAggregator for HavanaSampler {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != self.continuous_dims {
            return Err(BuildError::build(format!(
                "havana sampler expects continuous_dims={}, got {}",
                self.continuous_dims, point_spec.continuous_dims
            )));
        }
        if point_spec.discrete_dims != 0 {
            return Err(BuildError::build(format!(
                "havana sampler expects discrete_dims=0, got {}",
                point_spec.discrete_dims
            )));
        }
        Ok(())
    }

    fn is_training_active(&self) -> bool {
        HavanaSampler::is_training_active(self)
    }

    fn get_max_samples(&self) -> Option<usize> {
        if self.is_training_active() {
            Some(self.remaining_training_samples())
        } else {
            None
        }
    }

    fn produce_batch(
        &mut self,
        nr_samples: usize,
    ) -> Result<(crate::Batch, Option<BatchContext>), crate::engines::EngineError> {
        let should_train = self.is_training_active();
        let mut coords: Vec<f64> = Vec::with_capacity(nr_samples * self.continuous_dims);

        let context = if should_train {
            let mut samples = Vec::with_capacity(nr_samples);
            for _ in 0..nr_samples {
                let mut sample = Sample::new();
                self.grid.sample(&mut self.rng, &mut sample);

                match &sample {
                    Sample::Continuous(_weight, x) => {
                        debug_assert_eq!(x.len(), self.continuous_dims);
                        coords.extend_from_slice(x);
                    }
                    _ => unreachable!("continuous grid produced non-continuous sample"),
                }

                samples.push(sample);
            }
            Some(Box::new(HavanaBatchContext { samples }) as BatchContext)
        } else {
            for _ in 0..nr_samples {
                let mut sample = Sample::new();
                self.grid.sample(&mut self.rng, &mut sample);

                match &sample {
                    Sample::Continuous(_weight, x) => {
                        debug_assert_eq!(x.len(), self.continuous_dims);
                        coords.extend_from_slice(x);
                    }
                    _ => unreachable!("continuous grid produced non-continuous sample"),
                }
            }
            None
        };

        let batch = Batch::from_flat_data(nr_samples, self.continuous_dims, 0, coords, vec![])
            .map_err(|err| EngineError::engine(err.to_string()))?;
        self.batches_produced += 1;
        self.samples_produced = self.samples_produced.saturating_add(nr_samples);
        Ok((batch, context))
    }

    fn ingest_training_weights(
        &mut self,
        training_weights: &[f64],
        context: Option<BatchContext>,
    ) -> Result<(), crate::engines::EngineError> {
        let Some(context) = context else {
            // Training is disabled for this batch or context is unavailable.
            return Ok(());
        };
        let context = context
            .downcast::<HavanaBatchContext>()
            .map_err(|_| EngineError::engine("unexpected context type for Havana sampler"))?;

        if training_weights.len() != context.samples.len() {
            return Err(EngineError::engine(format!(
                "training/context size mismatch in Havana sampler: weights={}, samples={}",
                training_weights.len(),
                context.samples.len()
            )));
        }

        let before_samples_ingested = self.samples_ingested;
        let remaining_training = self.remaining_training_samples();
        let train_len = remaining_training.min(training_weights.len());
        for (eval, sample) in training_weights
            .iter()
            .zip(context.samples.iter())
            .take(train_len)
        {
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
            "training_rate": self.current_training_rate(),
        })
    }
}

struct HavanaBatchContext {
    pub samples: Vec<Sample<f64>>,
}
