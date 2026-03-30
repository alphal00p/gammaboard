use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, BatchTransform, Point};
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub struct SphericalBatchTransform;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SphericalBatchTransformParams {}

impl SphericalBatchTransform {
    pub fn from_params(_params: SphericalBatchTransformParams) -> Self {
        Self
    }
}

impl BatchTransform for SphericalBatchTransform {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        if domain.fixed_continuous_dims() != Some(3) {
            return Err(BuildError::build(
                "spherical batch transform requires continuous_dims == 3",
            ));
        }
        Ok(())
    }

    fn apply(&self, batch: Batch) -> Result<Batch, EngineError> {
        let dims = batch
            .points()
            .first()
            .map(|point| point.continuous.len())
            .unwrap_or(0);
        if dims != 3 {
            return Err(EngineError::engine(
                "spherical batch transform requires exactly 3 continuous dimensions",
            ));
        }

        let mut transformed_points = Vec::with_capacity(batch.size());
        for (row_idx, point) in batch.points().iter().enumerate() {
            if point.continuous.len() != 3 {
                return Err(EngineError::engine(format!(
                    "spherical batch transform requires homogeneous 3D points, got {} at row {}",
                    point.continuous.len(),
                    row_idx
                )));
            }
            let u_r = point.continuous[0];
            let u_theta = point.continuous[1];
            let u_phi = point.continuous[2];
            for (dim_idx, value) in [u_r, u_theta, u_phi].into_iter().enumerate() {
                if !(0.0..1.0).contains(&value) {
                    return Err(EngineError::engine(format!(
                        "spherical batch transform expects [0,1) inputs; row={row_idx} dim={dim_idx} value={value}"
                    )));
                }
            }

            let one_minus_u_r = 1.0 - u_r;
            if one_minus_u_r <= 0.0 {
                return Err(EngineError::engine(format!(
                    "spherical batch transform has singular radial map at u_r=1 (row={row_idx})"
                )));
            }
            let r = u_r / one_minus_u_r;
            let dr_du_r = 1.0 / (one_minus_u_r * one_minus_u_r);

            let cos_theta = 2.0 * u_theta - 1.0;
            let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
            let phi = 2.0 * PI * u_phi;
            let (sin_phi, cos_phi) = phi.sin_cos();
            let transformed_continuous = vec![
                r * sin_theta * cos_phi,
                r * sin_theta * sin_phi,
                r * cos_theta,
            ];

            let jacobian = 4.0 * PI * r * r * dr_du_r;
            transformed_points.push(Point::new(
                transformed_continuous,
                point.discrete.clone(),
                point.weight * jacobian.abs(),
            ));
        }

        Batch::new(transformed_points).map_err(|err| EngineError::engine(err.to_string()))
    }
}
