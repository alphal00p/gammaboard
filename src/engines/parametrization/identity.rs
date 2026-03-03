use super::Parametrization;
use crate::batch::Batch;
use crate::engines::{BuildError, BuildFromJson, EngineError};
use serde::Deserialize;

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
