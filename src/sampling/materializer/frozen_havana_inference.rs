use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Materializer, PointSpec};
use crate::sampling::{LatentBatch, LatentBatchPayload, SamplerAggregatorSnapshot, StageHandoff};
use crate::utils::rng::SerializableMonteCarloRng;
use serde::Deserialize;
use symbolica::numerical_integration::{Grid, Sample};

pub struct HavanaInferenceMaterializer {
    continuous_dims: usize,
    grid: Grid<f64>,
}

impl HavanaInferenceMaterializer {
    pub fn new(handoff: Option<StageHandoff<'_>>) -> Result<Self, BuildError> {
        let handoff = handoff.unwrap_or_default();

        // Accept either a HavanaTraining snapshot (which contains the grid) or a
        // HavanaInference snapshot that has been persisted with a grid. This keeps
        // materializer construction simple and compatible with both snapshot kinds.
        let raw = match handoff.sampler_snapshot {
            Some(SamplerAggregatorSnapshot::HavanaTraining { raw }) => raw.clone(),
            Some(SamplerAggregatorSnapshot::HavanaInference { raw }) => raw.clone(),
            _ => {
                return Err(BuildError::build(
                    "havana inference materializer requires a havana training or inference sampler snapshot containing a grid",
                ));
            }
        };

        #[derive(Deserialize)]
        struct GridOnlySnapshot {
            grid: serde_json::Value,
        }
        let grid_only: GridOnlySnapshot = serde_json::from_value(raw.clone()).map_err(|err| {
            BuildError::build(format!(
                "failed to decode havana sampler snapshot grid for materializer handoff: {err}"
            ))
        })?;
        let grid: Grid<f64> = serde_json::from_value(grid_only.grid).map_err(|err| {
            BuildError::build(format!(
                "failed to decode havana grid for materializer handoff: {err}"
            ))
        })?;

        let continuous_dims = match &grid {
            Grid::Continuous(grid) => grid.continuous_dimensions.len(),
            Grid::Discrete(_) | Grid::Uniform(_, _) => {
                return Err(BuildError::build(
                    "havana inference materializer requires a continuous grid",
                ));
            }
        };

        Ok(Self {
            continuous_dims,
            grid,
        })
    }
}

impl Materializer for HavanaInferenceMaterializer {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.discrete_dims != 0 {
            return Err(BuildError::build(
                "havana inference materializer requires point_spec.discrete_dims == 0",
            ));
        }
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        match &latent_batch.payload {
            LatentBatchPayload::HavanaInference { seed } => {
                let mut rng = SerializableMonteCarloRng::new(*seed, 0);
                let mut coords = Vec::with_capacity(latent_batch.nr_samples * self.continuous_dims);
                let mut weights = Vec::with_capacity(latent_batch.nr_samples);

                for _ in 0..latent_batch.nr_samples {
                    let mut sample = Sample::new();
                    self.grid.sample(&mut rng, &mut sample);
                    match sample {
                        Sample::Continuous(weight, x) => {
                            coords.extend_from_slice(&x);
                            weights.push(weight);
                        }
                        _ => {
                            return Err(EngineError::engine(
                                "havana inference materializer expected continuous samples",
                            ));
                        }
                    }
                }

                Batch::from_flat_data_with_weights(
                    latent_batch.nr_samples,
                    self.continuous_dims,
                    0,
                    coords,
                    Vec::new(),
                    Some(weights),
                )
                .map_err(|err| EngineError::engine(err.to_string()))
            }
            LatentBatchPayload::Batch { batch } => {
                // If the latent payload is already a concrete batch, accept it directly.
                Batch::from_json(batch).map_err(|err| EngineError::engine(err.to_string()))
            }
        }
    }
}
