use crate::core::EngineResultExt;
use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, Materializer};
use crate::sampling::havana_grid::{sample_to_point, validate_havana_grid_domain};
use crate::sampling::{LatentBatch, LatentBatchPayload, SamplerAggregatorSnapshot, StageHandoff};
use crate::utils::domain::Domain;
use serde::Deserialize;
use symbolica::numerical_integration::{Grid, Sample};

pub struct HavanaInferenceMaterializer {
    grid: Grid<f64>,
}

impl HavanaInferenceMaterializer {
    pub fn new(handoff: Option<StageHandoff<'_>>) -> Result<Self, BuildError> {
        crate::activate_symbolica_oem_license()
            .map_err(|err| BuildError::build(err.to_string()))?;
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
        let (state, offset) = match &latent_batch.payload {
            LatentBatchPayload::HavanaInference { rng_state } => (rng_state, 0),
            LatentBatchPayload::HavanaInferenceIndexed { rng_states, offset } => {
                latent_batch.validate_nr_samples().engine_err()?;
                (&rng_states[0], *offset)
            }
            LatentBatchPayload::IndexedBatch { .. } => {
                return latent_batch.payload.as_batch().engine_err();
            }
        };
        let mut rng = state.clone();
        let mut sample = Sample::new();
        for _ in 0..offset {
            self.grid.sample(&mut rng, &mut sample);
        }
        let mut points = Vec::with_capacity(latent_batch.nr_samples);
        for _ in 0..latent_batch.nr_samples {
            self.grid.sample(&mut rng, &mut sample);
            points.push(sample_to_point(&sample)?);
        }
        Batch::new(points).engine_err()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{AccumulatorConfig, SamplerAggregatorConfig};
    use crate::sampling::{HavanaInferenceSamplerParams, HavanaSamplerParams};

    #[test]
    fn havana_inference_materializer_emits_discrete_points() {
        let domain = Domain::rectangular(2, 1);
        let params = HavanaSamplerParams {
            generation_batch_size: 1_048_576,
            seed: 7,
            bins: 8,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let mut training = SamplerAggregatorConfig::HavanaTraining {
            params,
            materializer: None,
        }
        .build(domain.clone(), Some(8), None, serde_json::json!({}))
        .expect("build havana training sampler");
        let _ = training
            .generate(Some(4))
            .and_then(|generated| generated.into_batch())
            .expect("produce training batch");
        training
            .feedback(&[1.0, 2.0, 3.0, 4.0])
            .expect("ingest training weights");

        let snapshot = training.snapshot().expect("snapshot");
        let mut inference = SamplerAggregatorConfig::HavanaInference {
            params: HavanaInferenceSamplerParams::default(),
            materializer: None,
        }
        .build(
            domain.clone(),
            None,
            Some(crate::sampling::StageHandoff {
                sampler_snapshot: Some(&snapshot),
                observable_state: None,
            }),
            serde_json::json!({}),
        )
        .expect("build inference sampler");
        let latent_batch = inference
            .generate(Some(8))
            .and_then(|generated| generated.into_batch())
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
                accumulator: AccumulatorConfig::scalar(),
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
