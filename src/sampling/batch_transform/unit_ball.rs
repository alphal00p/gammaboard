use crate::core::{BuildError, EngineError};
use crate::evaluation::{Batch, BatchTransform, Point};
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

pub struct UnitBallBatchTransform;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UnitBallBatchTransformParams {}

impl UnitBallBatchTransform {
    pub fn from_params(_params: UnitBallBatchTransformParams) -> Self {
        Self
    }
}

impl BatchTransform for UnitBallBatchTransform {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError> {
        let continuous_dims = domain.fixed_continuous_dims().ok_or_else(|| {
            BuildError::build("unit_ball batch transform requires a fixed continuous dimension")
        })?;
        if continuous_dims == 0 {
            return Err(BuildError::build(
                "unit_ball batch transform requires continuous_dims >= 1",
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
        if dims == 0 {
            return Err(EngineError::engine(
                "unit_ball batch transform requires at least one continuous dimension",
            ));
        }

        let mut transformed_points = Vec::with_capacity(batch.size());
        for (row_idx, point) in batch.points().iter().enumerate() {
            if point.continuous.len() != dims {
                return Err(EngineError::engine(format!(
                    "unit_ball batch transform requires homogeneous continuous dimensions, got {} and {}",
                    dims,
                    point.continuous.len()
                )));
            }
            let mut unit = Vec::with_capacity(dims);
            for (dim_idx, value) in point.continuous.iter().copied().enumerate() {
                if !(0.0..=1.0).contains(&value) {
                    return Err(EngineError::engine(format!(
                        "unit_ball batch transform expects unit-hypercube inputs; row={row_idx} dim={dim_idx} value={value}"
                    )));
                }
                unit.push(value);
            }

            let (mapped, jacobian) = unit_hypercube_to_unit_ball(&unit);
            transformed_points.push(Point::new(
                mapped,
                point.discrete.clone(),
                point.weight * jacobian.abs(),
            ));
        }

        Batch::new(transformed_points).map_err(|err| EngineError::engine(err.to_string()))
    }
}

fn unit_hypercube_to_unit_ball(unit: &[f64]) -> (Vec<f64>, f64) {
    let dims = unit.len();
    if dims == 1 {
        return (vec![2.0 * unit[0] - 1.0], 2.0);
    }

    let r = unit[0];
    let mut angles = vec![0.0_f64; dims - 1];
    for angle_idx in 0..(dims - 1) {
        let source = unit[angle_idx + 1];
        angles[angle_idx] = if angle_idx == dims - 2 {
            2.0 * PI * source
        } else {
            PI * source
        };
    }

    let mut mapped = vec![0.0_f64; dims];
    let mut sin_product = 1.0_f64;
    for coord_idx in 0..(dims - 1) {
        mapped[coord_idx] = r * sin_product * angles[coord_idx].cos();
        sin_product *= angles[coord_idx].sin();
    }
    mapped[dims - 1] = r * sin_product;

    let mut jacobian = 2.0 * PI.powi((dims - 1) as i32) * r.powi((dims - 1) as i32);
    for angle_idx in 0..(dims - 2) {
        jacobian *= angles[angle_idx].sin().powi((dims - 2 - angle_idx) as i32);
    }

    (mapped, jacobian)
}
