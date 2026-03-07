use super::Parametrization;
use crate::core::Batch;
use crate::engines::{BuildError, BuildFromJson, EngineError};
use serde::Deserialize;

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
