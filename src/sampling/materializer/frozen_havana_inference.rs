use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Materializer, Point};
use crate::sampling::{LatentBatch, LatentBatchPayload, SamplerAggregatorSnapshot, StageHandoff};
use crate::utils::domain::Domain;
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
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        let (continuous_dims, discrete_dims) =
            domain.fixed_rectangular_dims().ok_or_else(|| {
                BuildError::build(
                    "havana inference materializer requires a fixed rectangular domain",
                )
            })?;
        if discrete_dims != 0 {
            return Err(BuildError::build(
                "havana inference materializer requires discrete_dims == 0",
            ));
        }
        if continuous_dims != self.continuous_dims {
            return Err(BuildError::build(format!(
                "havana inference materializer expects continuous_dims={}, got {}",
                self.continuous_dims, continuous_dims
            )));
        }
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        match &latent_batch.payload {
            LatentBatchPayload::HavanaInference { seed } => {
                let mut rng = SerializableMonteCarloRng::new(*seed, 0);
                let mut points = Vec::with_capacity(latent_batch.nr_samples);

                for _ in 0..latent_batch.nr_samples {
                    let mut sample = Sample::new();
                    self.grid.sample(&mut rng, &mut sample);
                    match sample {
                        Sample::Continuous(weight, x) => {
                            points.push(Point::new(x, Vec::new(), weight));
                        }
                        _ => {
                            return Err(EngineError::engine(
                                "havana inference materializer expected continuous samples",
                            ));
                        }
                    }
                }

                Batch::new(points).map_err(|err| EngineError::engine(err.to_string()))
            }
            LatentBatchPayload::Batch { batch } => {
                // If the latent payload is already a concrete batch, accept it directly.
                Batch::from_json(batch).map_err(|err| EngineError::engine(err.to_string()))
            }
        }
    }
}
