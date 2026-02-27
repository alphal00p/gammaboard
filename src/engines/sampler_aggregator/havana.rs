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
    batches_for_update: usize,
    stop_training_after_n_batches: Option<usize>,
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
            batches_for_update: 10,
            stop_training_after_n_batches: None,
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
    if parsed.batches_for_update == 0 {
        return Err(BuildError::build(
            "havana sampler requires batches_for_update > 0",
        ));
    }
    if parsed.stop_training_after_n_batches.is_none() {
        return Err(BuildError::build(
            "havana sampler requires stop_training_after_n_batches",
        ));
    }
    if parsed.stop_training_after_n_batches == Some(0) {
        return Err(BuildError::build(
            "havana sampler stop_training_after_n_batches must be > 0",
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
    batches_ingested: usize,
    batches_for_update: usize,
    stop_training_after_n_batches: usize,
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
        batches_for_update: usize,
        stop_training_after_n_batches: usize,
        initial_training_rate: f64,
        final_training_rate: f64,
    ) -> Self {
        Self {
            continuous_dims,
            batches_produced: 0,
            batches_ingested: 0,
            batches_for_update,
            stop_training_after_n_batches,
            initial_training_rate,
            final_training_rate,
            grid,
            rng,
        }
    }

    fn max_batches_for_cycle(&self) -> usize {
        let produced_cycle = self.batches_produced / self.batches_for_update;
        let ingested_cycle = self.batches_ingested / self.batches_for_update;

        if produced_cycle > ingested_cycle {
            return 0;
        }

        let produced_mod = self.batches_produced % self.batches_for_update;
        self.batches_for_update.saturating_sub(produced_mod)
    }

    fn is_training_active(&self) -> bool {
        self.batches_produced < self.stop_training_after_n_batches
    }

    fn current_training_rate(&self) -> f64 {
        let progress = (self
            .batches_produced
            .min(self.stop_training_after_n_batches) as f64)
            / (self.stop_training_after_n_batches as f64);
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
        let stop_training_after_n_batches =
            params.stop_training_after_n_batches.ok_or_else(|| {
                BuildError::build("havana sampler requires stop_training_after_n_batches")
            })?;

        Ok(HavanaSampler::new(
            params.continuous_dims,
            grid,
            rng,
            params.batches_for_update,
            stop_training_after_n_batches,
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

    fn get_max_batches(&self) -> Option<usize> {
        if self.is_training_active() {
            Some(self.max_batches_for_cycle())
        } else {
            None
        }
    }

    fn produce_batch(
        &mut self,
        nr_samples: usize,
    ) -> Result<(crate::Batch, Option<BatchContext>), crate::engines::EngineError> {
        let should_train = self.is_training_active();
        let mut samples = if should_train {
            Some(Vec::with_capacity(nr_samples))
        } else {
            None
        };
        let mut coords: Vec<f64> = Vec::with_capacity(nr_samples * self.continuous_dims);

        if should_train && self.max_batches_for_cycle() == 0 {
            return Err(EngineError::Engine(
                "tried producing batches before update".to_string(),
            ));
        }

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

            if let Some(train_samples) = samples.as_mut() {
                train_samples.push(sample);
            }
        }

        let batch = Batch::from_flat_data(nr_samples, self.continuous_dims, 0, coords, vec![])
            .map_err(|err| EngineError::engine(err.to_string()))?;
        let context =
            samples.map(|samples| Box::new(HavanaBatchContext { samples }) as BatchContext);
        self.batches_produced += 1;
        if self.batches_produced == self.stop_training_after_n_batches {
            info!(
                batches_produced = self.batches_produced,
                stop_training_after_n_batches = self.stop_training_after_n_batches,
                "havana sampler training complete"
            );
        }
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

        for (eval, sample) in training_weights.iter().zip(context.samples.iter()) {
            self.grid
                .add_training_sample(sample, *eval / sample.get_weight()) // the evaluator return the weighted eval, so it needs to be divided by the sample weight
                .map_err(|err| EngineError::engine(err.to_string()))?;
        }
        self.batches_ingested += 1;

        if self.batches_ingested % self.batches_for_update == 0 {
            let training_rate = self.current_training_rate();
            self.grid.update(training_rate, training_rate);
        }
        Ok(())
    }

    fn get_diagnostics(&mut self) -> serde_json::Value {
        let chi_sq = self.grid.get_statistics().chi_sq;
        json!({
            "chi_sq": chi_sq,
            "batches_produced": self.batches_produced,
            "batches_ingested": self.batches_ingested,
            "training_rate": self.current_training_rate(),
        })
    }
}

struct HavanaBatchContext {
    pub samples: Vec<Sample<f64>>,
}
