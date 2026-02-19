use serde_json::Value as JsonValue;
use symbolica::numerical_integration::{ContinuousGrid, Grid, MonteCarloRng};

use crate::{
    batch::PointSpec,
    engines::{BuildError, SamplerAggregatorEngine},
};

struct HavanaSampler {
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

impl SamplerAggregatorEngine for HavanaSampler {
    fn from_params(params: &JsonValue) -> Result<Self, BuildError>
    where
        Self: Sized,
    {
        let seed = params
            .get("seed")
            .and_then(JsonValue::as_u64)
            .unwrap_or_default();
        let batch_size = params
            .get("batch_size")
            .and_then(JsonValue::as_u64)
            .unwrap_or(64) as usize;
        let rng = MonteCarloRng::new(seed, 0);

        let n_dims = params
            .get("dims")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| BuildError::build("could not parse dims from params"))?
            as usize;
        let n_bins = params
            .get("bins")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| BuildError::build("could not parse bins from params"))?
            as usize;
        let min_samples_for_update = params
            .get("min_samples_for_update")
            .and_then(JsonValue::as_u64)
            .ok_or_else(|| {
                BuildError::build("could not parse min_samples_for_update from params")
            })? as usize;

        let grid = Grid::Continuous(ContinuousGrid::new(
            n_dims,
            n_bins,
            min_samples_for_update,
            None,
            false,
        ));

        Ok(HavanaSampler::new(n_dims, grid, rng, batch_size))
    }

    fn implementation(&self) -> &'static str {
        "havana"
    }

    fn version(&self) -> &'static str {
        "v1"
    }

    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != self.continuous_dims {
            return Err(BuildError::build(format!(
                "havana sampler expects continuous_dims={}, got {}",
                self.continuous_dims, point_spec.continuous_dims
            )));
        }
        Ok(())
    }

    fn init(
        &mut self,
        _state: Option<crate::engines::EngineState>,
    ) -> Result<(), crate::engines::EngineError> {
        let _ = (&self.batch_size, &self.grid, &self.rng);
        todo!()
    }

    fn produce_batches(
        &mut self,
        _max_batches: usize,
    ) -> Result<Vec<crate::Batch>, crate::engines::EngineError> {
        let _ = (&self.batch_size, &self.grid, &self.rng);
        todo!()
    }

    fn ingest_training_weights(
        &mut self,
        _training_weights: &[f64],
    ) -> Result<(), crate::engines::EngineError> {
        let _ = (&self.batch_size, &self.grid, &self.rng);
        todo!()
    }
}
