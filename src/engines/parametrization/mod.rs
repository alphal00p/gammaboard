use super::{BuildError, BuildFromJson, EngineError};
use crate::batch::{Batch, PointSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use strum::{AsRefStr, Display};

mod identity;
mod none;
mod spherical;
mod unit_ball;

use identity::IdentityParametrization;
use none::NoParametrization;
use spherical::SphericalParametrization;
use unit_ball::UnitBallParametrization;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ParametrizationImplementation {
    None,
    Identity,
    UnitBall,
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
            ParametrizationImplementation::UnitBall => {
                Ok(Box::new(UnitBallParametrization::from_json(&self.params)?))
            }
            ParametrizationImplementation::Spherical => {
                Ok(Box::new(SphericalParametrization::from_json(&self.params)?))
            }
        }
    }
}
