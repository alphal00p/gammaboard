use super::{BuildError, BuildFromJson, EngineError};
use crate::core::{Batch, PointSpec};
use serde_json::Value as JsonValue;

mod identity;
mod none;
mod spherical;
mod unit_ball;

use identity::IdentityParametrization;
use none::NoParametrization;
use spherical::SphericalParametrization;
use unit_ball::UnitBallParametrization;

pub trait Parametrization: Send + Sync {
    fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
        Ok(())
    }

    fn transform_batch(&mut self, batch: &Batch) -> Result<Batch, EngineError>;
}

impl crate::engines::ParametrizationConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::None { .. } => "none",
            Self::Identity { .. } => "identity",
            Self::UnitBall { .. } => "unit_ball",
            Self::Spherical { .. } => "spherical",
        }
    }

    pub fn build(&self) -> Result<Box<dyn Parametrization>, BuildError> {
        match self {
            Self::None { params } => Ok(Box::new(NoParametrization::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
            Self::Identity { params } => Ok(Box::new(IdentityParametrization::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
            Self::UnitBall { params } => Ok(Box::new(UnitBallParametrization::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
            Self::Spherical { params } => Ok(Box::new(SphericalParametrization::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
        }
    }
}
