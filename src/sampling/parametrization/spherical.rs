use super::Parametrization;
use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, PointSpec};
use crate::sampling::{LatentBatch, ParametrizationSnapshot};
use ndarray::{Array1, Array2};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub struct SphericalParametrization;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SphericalParametrizationParams {}

impl SphericalParametrization {
    pub fn from_params(_params: SphericalParametrizationParams) -> Self {
        Self
    }
}

impl Parametrization for SphericalParametrization {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims != 3 {
            return Err(BuildError::build(
                "spherical parametrization requires continuous_dims == 3",
            ));
        }
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        let batch = latent_batch
            .payload
            .as_batch()
            .map_err(|err| EngineError::engine(err.to_string()))?;
        let rows = batch.size();
        let dims = batch.continuous().ncols();
        if dims != 3 {
            return Err(EngineError::engine(
                "spherical parametrization requires exactly 3 continuous dimensions",
            ));
        }

        let mut transformed_continuous = Array2::<f64>::zeros((rows, 3));
        let mut transformed_weights = Array1::<f64>::zeros(rows);

        for (row_idx, row) in batch.continuous().rows().into_iter().enumerate() {
            let u_r = row[0];
            let u_theta = row[1];
            let u_phi = row[2];
            for (dim_idx, value) in [u_r, u_theta, u_phi].into_iter().enumerate() {
                if !(0.0..1.0).contains(&value) {
                    return Err(EngineError::engine(format!(
                        "spherical parametrization expects [0,1) inputs; row={row_idx} dim={dim_idx} value={value}"
                    )));
                }
            }

            let one_minus_u_r = 1.0 - u_r;
            if one_minus_u_r <= 0.0 {
                return Err(EngineError::engine(format!(
                    "spherical parametrization has singular radial map at u_r=1 (row={row_idx})"
                )));
            }
            let r = u_r / one_minus_u_r;
            let dr_du_r = 1.0 / (one_minus_u_r * one_minus_u_r);

            let cos_theta = 2.0 * u_theta - 1.0;
            let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
            let phi = 2.0 * PI * u_phi;
            let (sin_phi, cos_phi) = phi.sin_cos();

            transformed_continuous[(row_idx, 0)] = r * sin_theta * cos_phi;
            transformed_continuous[(row_idx, 1)] = r * sin_theta * sin_phi;
            transformed_continuous[(row_idx, 2)] = r * cos_theta;

            let jacobian = 4.0 * PI * r * r * dr_du_r;
            transformed_weights[row_idx] = batch.weights()[row_idx] * jacobian.abs();
        }

        Batch::new(
            transformed_continuous,
            batch.discrete().clone(),
            Some(transformed_weights),
        )
        .map_err(|err| EngineError::engine(err.to_string()))
    }

    fn snapshot(&self) -> Result<ParametrizationSnapshot, EngineError> {
        Ok(ParametrizationSnapshot::Spherical {})
    }
}
