use serde::Deserialize;
use serde_json::json;
use symbolica::numerical_integration::{ContinuousGrid, Grid, MonteCarloRng, Sample};

use crate::{
    Batch, EngineError,
    batch::PointSpec,
    engines::{BatchContext, BuildError, BuildFromJson, SamplerAggregator},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaSamplerParams {
    seed: u64,
    batch_size: usize,
    continuous_dims: usize,
    bins: usize,
    min_samples_for_update: usize,
    batches_for_update: usize,
    stop_training_after_n_batches: Option<usize>,
    learning_rate: f64,
}

impl Default for HavanaSamplerParams {
    fn default() -> Self {
        Self {
            seed: 0,
            batch_size: 64,
            continuous_dims: 1,
            bins: 64,
            min_samples_for_update: 1_024,
            batches_for_update: 10,
            stop_training_after_n_batches: None,
            learning_rate: 0.1,
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
    if parsed.batch_size == 0 {
        return Err(BuildError::build("havana sampler requires batch_size > 0"));
    }
    if parsed.batches_for_update == 0 {
        return Err(BuildError::build(
            "havana sampler requires batches_for_update > 0",
        ));
    }
    if parsed.stop_training_after_n_batches == Some(0) {
        return Err(BuildError::build(
            "havana sampler stop_training_after_n_batches must be > 0 when set",
        ));
    }

    Ok(())
}

pub struct HavanaSampler {
    batch_size: usize,
    continuous_dims: usize,
    batches_produced_since_update: usize,
    batches_ingested_since_update: usize,
    total_batches_produced: usize,
    batches_for_update: usize,
    stop_training_after_n_batches: Option<usize>,
    learning_rate: f64,
    grid: Grid<f64>,
    rng: MonteCarloRng,
}

impl HavanaSampler {
    fn new(
        continuous_dims: usize,
        grid: Grid<f64>,
        rng: MonteCarloRng,
        batch_size: usize,
        batches_for_update: usize,
        stop_training_after_n_batches: Option<usize>,
        learning_rate: f64,
    ) -> Self {
        Self {
            batch_size,
            continuous_dims,
            batches_produced_since_update: 0,
            batches_ingested_since_update: 0,
            total_batches_produced: 0,
            batches_for_update,
            stop_training_after_n_batches,
            learning_rate,
            grid,
            rng,
        }
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

        Ok(HavanaSampler::new(
            params.continuous_dims,
            grid,
            rng,
            params.batch_size,
            params.batches_for_update,
            params.stop_training_after_n_batches,
            params.learning_rate,
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
        let remaining_until_update = self
            .batches_for_update
            .saturating_sub(self.batches_produced_since_update);
        let remaining_until_stop = self
            .stop_training_after_n_batches
            .map(|limit| limit.saturating_sub(self.total_batches_produced))
            .unwrap_or(usize::MAX);
        Some(remaining_until_update.min(remaining_until_stop))
    }

    fn produce_batch(
        &mut self,
        nr_samples: usize,
    ) -> Result<(crate::Batch, Option<BatchContext>), crate::engines::EngineError> {
        let mut samples = Vec::with_capacity(nr_samples);
        let mut coords: Vec<f64> = Vec::with_capacity(nr_samples * self.continuous_dims);

        if self.batches_produced_since_update >= self.batches_for_update {
            return Err(EngineError::Engine(
                "tried producing batches before update".to_string(),
            ));
        }
        if self
            .stop_training_after_n_batches
            .is_some_and(|limit| self.total_batches_produced >= limit)
        {
            return Err(EngineError::Engine(
                "havana sampler training batch limit reached".to_string(),
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

            samples.push(sample);
        }

        let batch = Batch::from_flat_data(nr_samples, self.continuous_dims, 0, coords, vec![])
            .map_err(|err| EngineError::engine(err.to_string()))?;
        let context: BatchContext = Box::new(HavanaBatchContext { samples });
        self.batches_produced_since_update += 1;
        self.total_batches_produced += 1;
        Ok((batch, Some(context)))
    }

    fn ingest_training_weights(
        &mut self,
        training_weights: &[f64],
        context: Option<BatchContext>,
    ) -> Result<(), crate::engines::EngineError> {
        let _ = (&self.batch_size, &self.rng);
        let context = context
            .ok_or_else(|| EngineError::engine("missing Havana batch context"))?
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
                .add_training_sample(sample, *eval)
                .map_err(|err| EngineError::engine(err.to_string()))?;
        }
        self.batches_ingested_since_update += 1;

        if self.batches_ingested_since_update >= self.batches_for_update {
            self.grid.update(self.learning_rate, self.learning_rate);
            self.batches_ingested_since_update = 0;
            self.batches_produced_since_update = 0;
        }
        Ok(())
    }

    fn get_diagnostics(&mut self) -> serde_json::Value {
        let chi_sq = self.grid.get_statistics().chi_sq;
        return json!({"chi_sq": chi_sq});
    }
}

struct HavanaBatchContext {
    pub samples: Vec<Sample<f64>>,
}
