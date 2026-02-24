use serde::Deserialize;
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
}

impl Default for HavanaSamplerParams {
    fn default() -> Self {
        Self {
            seed: 0,
            batch_size: 64,
            continuous_dims: 1,
            bins: 64,
            min_samples_for_update: 1_024,
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

    Ok(())
}

pub struct HavanaSampler {
    batch_size: usize,
    continuous_dims: usize,
    grid: Grid<f64>,
    rng: MonteCarloRng,
}

impl HavanaSampler {
    fn new(continuous_dims: usize, grid: Grid<f64>, rng: MonteCarloRng, batch_size: usize) -> Self {
        Self {
            batch_size,
            continuous_dims,
            grid,
            rng,
        }
    }
}

impl BuildFromJson for HavanaSampler {
    type Params = HavanaSamplerParams;
    const PARAMS_CONTEXT: &'static str = "havana sampler params";

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

    fn init(&mut self) -> Result<(), crate::engines::EngineError> {
        let _ = (&self.batch_size, &self.grid, &self.rng);
        todo!()
    }

    fn produce_batch(
        &mut self,
        _nr_samples: usize,
    ) -> Result<(crate::Batch, Option<BatchContext>), crate::engines::EngineError> {
        let mut samples = Vec::with_capacity(_nr_samples);
        let mut coords: Vec<f64> = Vec::with_capacity(_nr_samples * self.continuous_dims);

        for _ in 0.._nr_samples {
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

        let batch = Batch::from_flat_data(_nr_samples, self.continuous_dims, 0, coords, vec![])
            .map_err(|err| EngineError::engine(err.to_string()))?;
        let context: BatchContext = Box::new(HavanaBatchContext { samples });

        Ok((batch, Some(context)))
    }

    fn ingest_training_weights(
        &mut self,
        _training_weights: &[f64],
        _context: Option<BatchContext>,
    ) -> Result<(), crate::engines::EngineError> {
        let _ = (&self.batch_size, &self.rng);
        let context = _context
            .ok_or_else(|| EngineError::engine("missing Havana batch context"))?
            .downcast::<HavanaBatchContext>()
            .map_err(|_| EngineError::engine("unexpected context type for Havana sampler"))?;

        if _training_weights.len() != context.samples.len() {
            return Err(EngineError::engine(format!(
                "training/context size mismatch in Havana sampler: weights={}, samples={}",
                _training_weights.len(),
                context.samples.len()
            )));
        }

        for (eval, sample) in _training_weights.iter().zip(context.samples.iter()) {
            self.grid
                .add_training_sample(sample, *eval)
                .map_err(|err| EngineError::engine(err.to_string()))?;
        }
        Ok(())
    }
}

struct HavanaBatchContext {
    pub samples: Vec<Sample<f64>>,
}
