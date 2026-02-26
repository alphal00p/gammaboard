use super::{BuildError, BuildFromJson, EngineError};
use crate::batch::{Batch, PointSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use strum::{AsRefStr, Display};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ParametrizationImplementation {
    None,
    Identity,
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
