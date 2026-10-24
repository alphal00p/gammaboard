use crate::core::EngineError;
use crate::core::EngineResultExt;
use crate::evaluation::{Batch, Materializer};
use crate::sampling::havana_grid::{sample_to_point, validate_havana_grid_domain};
use crate::sampling::{LatentBatch, LatentBatchPayload, StageHandoff};
use crate::utils::domain::Domain;
use symbolica::numerical_integration::{Grid, Sample};

pub struct HavanaInferenceMaterializer {
    domain: Domain,
    grid: Grid<f64>,
}

impl HavanaInferenceMaterializer {
    pub fn new(handoff: Option<StageHandoff<'_>>) -> Result<Self, EngineError> {
        crate::activate_symbolica_oem_license()
            .map_err(|err| EngineError::build(err.to_string()))?;
        let handoff = handoff.unwrap_or_default();

        let snapshot = handoff
            .sampler_snapshot
            .filter(|s| s.contains_havana_grid())
            .ok_or_else(|| {
                EngineError::build("havana inference materializer requires a saved Havana grid")
            })?;
        let grid: Grid<f64> =
            serde_json::from_value(snapshot.state["grid"].clone()).map_err(|err| {
                EngineError::build(format!(
                    "failed to decode havana grid for materializer handoff: {err}"
                ))
            })?;

        let domain: Domain = serde_json::from_value(snapshot.state["domain"].clone())
            .map_err(|err| EngineError::build(format!("invalid Havana domain: {err}")))?;
        domain.validate().map_err(EngineError::build)?;
        validate_havana_grid_domain(&grid, &domain, "havana materializer")?;
        Ok(Self { grid, domain })
    }
}

impl Materializer for HavanaInferenceMaterializer {
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError> {
        validate_havana_grid_domain(&self.grid, domain, "havana inference materializer")
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        let (state, offset) = match &latent_batch.payload {
            LatentBatchPayload::HavanaInference { rng_states, offset } => {
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
            let mut point = sample_to_point(&sample)?;
            crate::sampling::havana_grid::map_discrete_path(
                &self.domain,
                &mut point.discrete,
                true,
            )?;
            points.push(point);
        }
        Ok(Batch::new(points))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SamplingConfig;
    use crate::sampling::{HavanaInferenceSamplerParams, HavanaSamplerParams};

    #[test]
    fn havana_inference_materializer_emits_discrete_points() {
        let domain = Domain::rectangular(2, 1);
        let params = HavanaSamplerParams {
            seed: 7,
            bins: 8,
            samples_for_update: 16,
            initial_training_rate: 0.1,
            final_training_rate: 0.01,
        };
        let training_config = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::HavanaTraining { params },
            materializer: None,
            transforms: None,
        };
        let mut training = training_config
            .create(domain.clone(), Some(8), serde_json::json!({}))
            .expect("build havana training sampler");
        let _ = training
            .generate(4)
            .and_then(|generated| generated.into_batch())
            .expect("produce training batch");
        training
            .feedback(&crate::sampling::Feedback::new(
                ([1.0, 2.0, 3.0, 4.0]).to_vec(),
            ))
            .expect("ingest training weights");

        let snapshot = training_config.snapshot(&mut *training).expect("snapshot");
        let mut invalid = snapshot.clone();
        invalid.version += 1;
        let invalid_handoff = StageHandoff {
            sampler_snapshot: Some(&invalid),
            observable_state: None,
        };
        assert!(HavanaInferenceMaterializer::new(Some(invalid_handoff)).is_err());
        assert!(
            SamplingConfig {
                stratification: None,
                sampler: crate::core::SamplerConfig::HavanaInference {
                    params: Default::default()
                },
                materializer: None,
                transforms: None
            }
            .from_stage(&domain, invalid_handoff)
            .is_err()
        );
        let mut inference = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::HavanaInference {
                params: HavanaInferenceSamplerParams::default(),
            },
            materializer: None,
            transforms: None,
        }
        .from_stage(
            &domain,
            crate::sampling::StageHandoff {
                sampler_snapshot: Some(&snapshot),
                observable_state: None,
            },
        )
        .expect("build inference sampler");
        let latent_batch = inference
            .generate(8)
            .and_then(|generated| generated.into_batch())
            .expect("produce inference batch");
        let snapshot = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::HavanaInference {
                params: Default::default(),
            },
            materializer: None,
            transforms: None,
        }
        .snapshot(&mut *inference)
        .expect("snapshot");

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
                payload: latent_batch.payload,
            })
            .expect("materialize batch");

        assert_eq!(batch.len(), 8);
        assert!(batch.points().iter().all(|point| point.discrete.len() == 1));
        assert!(
            batch
                .points()
                .iter()
                .all(|point| point.continuous.len() == 2)
        );
    }
}
