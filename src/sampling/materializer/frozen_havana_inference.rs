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

        let Some(SamplerAggregatorSnapshot::HavanaTraining { raw }) = handoff.sampler_snapshot
        else {
            return Err(BuildError::build(
                "havana inference materializer requires a havana training sampler snapshot",
            ));
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
        let LatentBatchPayload::HavanaInference { seed } = latent_batch.payload.clone() else {
            return Err(EngineError::engine(
                "havana inference materializer requires havana_inference latent payloads",
            ));
        };

        let mut rng = SerializableMonteCarloRng::new(seed, 0);
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
}
