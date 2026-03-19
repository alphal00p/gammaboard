use crate::core::{BuildError, EngineError, ParametrizationConfig};
use crate::evaluation::{Batch, Parametrization, PointSpec};
use crate::sampling::parametrization::ParametrizationSnapshot;
use crate::sampling::{LatentBatch, LatentBatchPayload, StageHandoff};
use crate::utils::rng::SerializableMonteCarloRng;
use serde::{Deserialize, Serialize};
use symbolica::numerical_integration::{Grid, Sample};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaInferenceParametrizationParams {
    pub inner: Box<ParametrizationConfig>,
}

pub struct HavanaInferenceParametrization {
    continuous_dims: usize,
    grid: Grid<f64>,
    inner: Box<dyn Parametrization>,
}

impl Default for HavanaInferenceParametrizationParams {
    fn default() -> Self {
        Self {
            inner: Box::new(ParametrizationConfig::Identity {
                params: Default::default(),
            }),
        }
    }
}

impl HavanaInferenceParametrization {
    pub fn from_build_context(
        params: HavanaInferenceParametrizationParams,
        handoff: Option<StageHandoff<'_>>,
    ) -> Result<Self, BuildError> {
        let handoff = handoff.unwrap_or_default();
        let (grid, inner_snapshot) = match handoff.parametrization_snapshot {
            Some(ParametrizationSnapshot::HavanaInference { grid, inner }) => {
                let grid = serde_json::from_value(grid.clone()).map_err(|err| {
                    BuildError::build(format!(
                        "failed to decode havana inference parametrization snapshot grid: {err}"
                    ))
                })?;
                (grid, Some(inner.as_ref()))
            }
            None => {
                let Some(crate::sampling::SamplerAggregatorSnapshot::HavanaTraining { raw }) =
                    handoff.sampler_snapshot
                else {
                    return Err(BuildError::build(
                        "havana inference parametrization requires a havana training sampler snapshot or a parametrization snapshot",
                    ));
                };
                #[derive(Deserialize)]
                struct GridOnlySnapshot {
                    grid: serde_json::Value,
                }
                let grid_only: GridOnlySnapshot =
                    serde_json::from_value(raw.clone()).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana sampler snapshot grid for parametrization handoff: {err}"
                        ))
                    })?;
                let grid = serde_json::from_value(grid_only.grid).map_err(|err| {
                    BuildError::build(format!(
                        "failed to decode havana grid for parametrization handoff: {err}"
                    ))
                })?;
                (grid, None)
            }
            Some(other_snapshot) => {
                let Some(crate::sampling::SamplerAggregatorSnapshot::HavanaTraining { raw }) =
                    handoff.sampler_snapshot
                else {
                    return Err(BuildError::build(format!(
                        "havana inference parametrization cannot restore from snapshot kind {other_snapshot:?}"
                    )));
                };
                #[derive(Deserialize)]
                struct GridOnlySnapshot {
                    grid: serde_json::Value,
                }
                let grid_only: GridOnlySnapshot =
                    serde_json::from_value(raw.clone()).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana sampler snapshot grid for parametrization handoff: {err}"
                        ))
                    })?;
                let grid = serde_json::from_value(grid_only.grid).map_err(|err| {
                    BuildError::build(format!(
                        "failed to decode havana grid for parametrization handoff: {err}"
                    ))
                })?;
                (grid, Some(other_snapshot))
            }
        };

        let continuous_dims = match &grid {
            Grid::Continuous(grid) => grid.continuous_dimensions.len(),
            Grid::Discrete(_) | Grid::Uniform(_, _) => {
                return Err(BuildError::build(
                    "havana inference parametrization requires a continuous grid",
                ));
            }
        };

        let inner = params.inner.build(Some(StageHandoff {
            parametrization_snapshot: inner_snapshot,
            ..StageHandoff::default()
        }))?;

        Ok(Self {
            continuous_dims,
            grid,
            inner,
        })
    }
}

impl Parametrization for HavanaInferenceParametrization {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.discrete_dims != 0 {
            return Err(BuildError::build(
                "havana inference parametrization requires point_spec.discrete_dims == 0",
            ));
        }
        self.inner.validate_point_spec(point_spec)?;
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        let LatentBatchPayload::HavanaInference { seed } = latent_batch.payload.clone() else {
            return Err(EngineError::engine(
                "havana inference parametrization requires havana_inference latent payloads",
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
                        "havana inference parametrization expected continuous samples",
                    ));
                }
            }
        }

        let batch = Batch::from_flat_data_with_weights(
            latent_batch.nr_samples,
            self.continuous_dims,
            0,
            coords,
            Vec::new(),
            Some(weights),
        )
        .map_err(|err| EngineError::engine(err.to_string()))?;
        let inner_latent = LatentBatch {
            nr_samples: latent_batch.nr_samples,
            observable: latent_batch.observable.clone(),
            payload: LatentBatchPayload::from_batch(&batch),
        };
        self.inner.materialize_batch(&inner_latent)
    }

    fn snapshot(&self) -> Result<ParametrizationSnapshot, EngineError> {
        let grid = serde_json::to_value(&self.grid).map_err(|err| {
            EngineError::engine(format!(
                "failed to serialize havana inference parametrization grid: {err}"
            ))
        })?;
        Ok(ParametrizationSnapshot::HavanaInference {
            grid,
            inner: Box::new(self.inner.snapshot()?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::parametrization::IdentityParametrizationParams;
    use crate::sampling::{SamplerAggregatorSnapshot, StageHandoff};
    use serde_json::json;
    use symbolica::numerical_integration::ContinuousGrid;

    #[test]
    fn havana_inference_restores_from_snapshot() {
        let params = HavanaInferenceParametrizationParams {
            inner: Box::new(ParametrizationConfig::Identity {
                params: IdentityParametrizationParams::default(),
            }),
        };
        let snapshot = ParametrizationSnapshot::HavanaInference {
            grid: serde_json::to_value(Grid::Continuous(ContinuousGrid::<f64>::new(
                2, 8, 4, None, false,
            )))
            .expect("serialize"),
            inner: Box::new(ParametrizationSnapshot::Identity {}),
        };
        let mut parametrization = HavanaInferenceParametrization::from_build_context(
            params,
            Some(StageHandoff {
                parametrization_snapshot: Some(&snapshot),
                ..StageHandoff::default()
            }),
        )
        .expect("build");
        parametrization
            .validate_point_spec(&PointSpec {
                continuous_dims: 2,
                discrete_dims: 0,
            })
            .expect("validate");

        let latent = LatentBatch {
            nr_samples: 4,
            observable: crate::core::ObservableConfig::Scalar,
            payload: LatentBatchPayload::HavanaInference { seed: 42 },
        };
        let batch = parametrization
            .materialize_batch(&latent)
            .expect("materialize");
        assert_eq!(batch.size(), 4);
    }

    #[test]
    fn havana_inference_builds_from_sampler_snapshot() {
        let params = HavanaInferenceParametrizationParams::default();
        let sampler_snapshot = SamplerAggregatorSnapshot::HavanaTraining {
            raw: json!({ "grid": Grid::Continuous(ContinuousGrid::<f64>::new(2, 8, 4, None, false)) }),
        };
        let parametrization = HavanaInferenceParametrization::from_build_context(
            params,
            Some(StageHandoff {
                sampler_snapshot: Some(&sampler_snapshot),
                ..StageHandoff::default()
            }),
        )
        .expect("build");
        assert_eq!(parametrization.continuous_dims, 2);
    }

    #[test]
    fn havana_inference_uses_previous_inner_snapshot_with_sampler_handoff() {
        let params = HavanaInferenceParametrizationParams::default();
        let sampler_snapshot = SamplerAggregatorSnapshot::HavanaTraining {
            raw: json!({ "grid": Grid::Continuous(ContinuousGrid::<f64>::new(2, 8, 4, None, false)) }),
        };
        let previous_inner_snapshot = ParametrizationSnapshot::Identity {};
        let parametrization = HavanaInferenceParametrization::from_build_context(
            params,
            Some(StageHandoff {
                sampler_snapshot: Some(&sampler_snapshot),
                parametrization_snapshot: Some(&previous_inner_snapshot),
                ..StageHandoff::default()
            }),
        )
        .expect("build");
        assert_eq!(parametrization.continuous_dims, 2);
    }
}
