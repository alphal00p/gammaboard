use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Materializer};
use crate::sampling::havana_grid::{sample_to_point, validate_havana_grid_domain};
use crate::sampling::{LatentBatch, LatentBatchPayload, SamplerAggregatorSnapshot, StageHandoff};
use crate::utils::domain::Domain;
use crate::utils::rng::SerializableMonteCarloRng;
use serde::Deserialize;
use symbolica::numerical_integration::{Grid, Sample};

pub struct HavanaInferenceMaterializer {
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

        Ok(Self { grid })
    }
}

impl Materializer for HavanaInferenceMaterializer {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        validate_havana_grid_domain(&self.grid, domain, "havana inference materializer")
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        match &latent_batch.payload {
            LatentBatchPayload::HavanaInference { seed } => {
                let mut rng = SerializableMonteCarloRng::new(*seed, 0);
                let mut points = Vec::with_capacity(latent_batch.nr_samples);

                for _ in 0..latent_batch.nr_samples {
                    let mut sample = Sample::new();
                    self.grid.sample(&mut rng, &mut sample);
                    points.push(sample_to_point(&sample)?);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{ObservableConfig, SamplerAggregatorConfig};
    use crate::sampling::{HavanaInferenceSamplerParams, HavanaSamplerParams};

    #[test]
    fn havana_inference_materializer_emits_discrete_points() {
        let domain = Domain::rectangular(2, 1);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            min_samples_for_update: 4,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut training = SamplerAggregatorConfig::HavanaTraining { params }
            .build(domain.clone(), Some(8), None)
            .expect("build havana training sampler");
        let _ = training
            .produce_latent_batch(4)
            .expect("produce training batch");
        training
            .ingest_training_weights(&[1.0, 2.0, 3.0, 4.0])
            .expect("ingest training weights");

        let snapshot = training.snapshot().expect("snapshot");
        let mut inference = SamplerAggregatorConfig::HavanaInference {
            params: HavanaInferenceSamplerParams::default(),
        }
        .build(
            domain.clone(),
            None,
            Some(crate::sampling::StageHandoff {
                sampler_snapshot: Some(&snapshot),
                observable_state: None,
            }),
        )
        .expect("build inference sampler");
        let latent_batch = inference
            .produce_latent_batch(8)
            .expect("produce inference batch");
        let snapshot = inference.snapshot().expect("inference snapshot");

        let handoff = crate::sampling::StageHandoffOwned {
            sampler_snapshot: Some(snapshot),
            observable_state: None,
        };
        let mut materializer =
            HavanaInferenceMaterializer::new(Some(handoff.as_ref())).expect("materializer");
        materializer
            .validate_domain(&domain)
            .expect("domain validation");
        let batch = materializer
            .materialize_batch(&crate::sampling::LatentBatch {
                nr_samples: latent_batch.nr_samples,
                observable: ObservableConfig::Scalar,
                payload: latent_batch.payload,
            })
            .expect("materialize batch");

        assert_eq!(batch.size(), 8);
        assert!(batch.points().iter().all(|point| point.discrete.len() == 1));
        assert!(
            batch
                .points()
                .iter()
                .all(|point| point.continuous.len() == 2)
        );
    }
}
