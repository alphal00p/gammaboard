use crate::core::{BuildError, EngineError, ParametrizationConfig};
use crate::evaluation::{Batch, Parametrization, PointSpec};
use crate::sampling::{LatentBatch, LatentBatchPayload};
use crate::utils::rng::SerializableMonteCarloRng;
use serde::{Deserialize, Serialize};
use symbolica::numerical_integration::{Grid, Sample};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaInferenceParametrizationParams {
    pub inner: Box<ParametrizationConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenHavanaInferenceParametrizationParams {
    pub grid: Grid<f64>,
    pub inner: Box<ParametrizationConfig>,
}

pub struct FrozenHavanaInferenceParametrization {
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

impl FrozenHavanaInferenceParametrization {
    pub fn from_params(
        params: FrozenHavanaInferenceParametrizationParams,
    ) -> Result<Self, BuildError> {
        let continuous_dims = match &params.grid {
            Grid::Continuous(grid) => grid.continuous_dimensions.len(),
            Grid::Discrete(_) | Grid::Uniform(_, _) => {
                return Err(BuildError::build(
                    "frozen havana inference parametrization requires a continuous grid",
                ));
            }
        };
        let inner = params.inner.build()?;
        Ok(Self {
            continuous_dims,
            grid: params.grid,
            inner,
        })
    }
}

impl Parametrization for FrozenHavanaInferenceParametrization {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.discrete_dims != 0 {
            return Err(BuildError::build(
                "frozen havana inference parametrization requires point_spec.discrete_dims == 0",
            ));
        }
        self.inner.validate_point_spec(point_spec)?;
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        let LatentBatchPayload::HavanaInference { seed } = latent_batch.payload.clone() else {
            return Err(EngineError::engine(
                "frozen havana inference parametrization requires havana_inference latent payloads",
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
                        "frozen havana inference parametrization expected continuous samples",
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
            parametrization_state_version: latent_batch.parametrization_state_version,
            payload: LatentBatchPayload::from_batch(&batch),
        };
        self.inner.materialize_batch(&inner_latent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::PointSpec;
    use crate::sampling::parametrization::IdentityParametrizationParams;
    use symbolica::numerical_integration::ContinuousGrid;

    #[test]
    fn frozen_havana_materializes_compact_inference_payload() {
        let params = FrozenHavanaInferenceParametrizationParams {
            grid: Grid::Continuous(ContinuousGrid::new(2, 8, 4, None, false)),
            inner: Box::new(ParametrizationConfig::Identity {
                params: IdentityParametrizationParams::default(),
            }),
        };
        let mut parametrization =
            FrozenHavanaInferenceParametrization::from_params(params).expect("build");
        parametrization
            .validate_point_spec(&PointSpec {
                continuous_dims: 2,
                discrete_dims: 0,
            })
            .expect("validate");

        let latent = LatentBatch {
            nr_samples: 4,
            parametrization_state_version: 1,
            payload: LatentBatchPayload::HavanaInference { seed: 42 },
        };
        let batch = parametrization
            .materialize_batch(&latent)
            .expect("materialize");
        assert_eq!(batch.size(), 4);
        assert_eq!(batch.continuous().ncols(), 2);
        assert_eq!(batch.discrete().ncols(), 0);
        assert_eq!(batch.weights().len(), 4);
    }
}
