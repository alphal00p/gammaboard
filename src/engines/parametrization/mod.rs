use super::{BuildError, BuildFromJson, EngineError};
use crate::batch::{Batch, PointSpec};
use ndarray::{Array1, Array2};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::f64::consts::PI;
use strum::{AsRefStr, Display};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ParametrizationImplementation {
    None,
    Identity,
    Spherical,
}

pub trait Parametrization: Send + Sync {
    fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
        Ok(())
    }

    fn transform_batch(&mut self, batch: &Batch) -> Result<Batch, EngineError>;
}

#[derive(Debug, Clone)]
pub struct ParametrizationFactory {
    implementation: ParametrizationImplementation,
    params: JsonValue,
}

impl ParametrizationFactory {
    pub fn new(implementation: ParametrizationImplementation, params: JsonValue) -> Self {
        Self {
            implementation,
            params,
        }
    }

    pub fn build(&self) -> Result<Box<dyn Parametrization>, BuildError> {
        match self.implementation {
            ParametrizationImplementation::None => {
                Ok(Box::new(NoParametrization::from_json(&self.params)?))
            }
            ParametrizationImplementation::Identity => {
                Ok(Box::new(IdentityParametrization::from_json(&self.params)?))
            }
            ParametrizationImplementation::Spherical => {
                Ok(Box::new(SphericalParametrization::from_json(&self.params)?))
            }
        }
    }
}

pub struct NoParametrization;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct NoParametrizationParams {}

impl BuildFromJson for NoParametrization {
    type Params = NoParametrizationParams;

    fn from_parsed_params(_params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self)
    }
}

impl Parametrization for NoParametrization {
    fn transform_batch(&mut self, batch: &Batch) -> Result<Batch, EngineError> {
        Ok(batch.clone())
    }
}

pub struct IdentityParametrization;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct IdentityParametrizationParams {}

impl BuildFromJson for IdentityParametrization {
    type Params = IdentityParametrizationParams;

    fn from_parsed_params(_params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self)
    }
}

impl Parametrization for IdentityParametrization {
    fn transform_batch(&mut self, batch: &Batch) -> Result<Batch, EngineError> {
        Ok(batch.clone())
    }
}

pub struct SphericalParametrization;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SphericalParametrizationParams {}

impl BuildFromJson for SphericalParametrization {
    type Params = SphericalParametrizationParams;

    fn from_parsed_params(_params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self)
    }
}

impl Parametrization for SphericalParametrization {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        if point_spec.continuous_dims == 0 {
            return Err(BuildError::build(
                "spherical parametrization requires continuous_dims >= 1",
            ));
        }
        Ok(())
    }

    fn transform_batch(&mut self, batch: &Batch) -> Result<Batch, EngineError> {
        let rows = batch.size();
        let dims = batch.continuous().ncols();
        if dims == 0 {
            return Err(EngineError::engine(
                "spherical parametrization requires at least one continuous dimension",
            ));
        }

        let mut transformed_continuous = Array2::<f64>::zeros((rows, dims));
        let mut transformed_weights = Array1::<f64>::zeros(rows);

        for (row_idx, row) in batch.continuous().rows().into_iter().enumerate() {
            let mut unit = Vec::with_capacity(dims);
            for (dim_idx, value) in row.iter().copied().enumerate() {
                if !(0.0..=1.0).contains(&value) {
                    return Err(EngineError::engine(format!(
                        "spherical parametrization expects unit-hypercube inputs; row={row_idx} dim={dim_idx} value={value}"
                    )));
                }
                unit.push(value);
            }

            let (mapped, jacobian) = unit_hypercube_to_unit_ball(&unit);
            for (dim_idx, value) in mapped.into_iter().enumerate() {
                transformed_continuous[(row_idx, dim_idx)] = value;
            }
            transformed_weights[row_idx] = batch.weights()[row_idx] * jacobian.abs();
        }

        Batch::new(
            transformed_continuous,
            batch.discrete().clone(),
            Some(transformed_weights),
        )
        .map_err(|err| EngineError::engine(err.to_string()))
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
