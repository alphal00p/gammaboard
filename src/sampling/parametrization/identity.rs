use super::Parametrization;
use crate::engines::{Batch, EngineError, LatentBatch};
use serde::{Deserialize, Serialize};

pub struct IdentityParametrization;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct IdentityParametrizationParams {}

impl IdentityParametrization {
    pub fn from_params(_params: IdentityParametrizationParams) -> Self {
        Self
    }
}

impl Parametrization for IdentityParametrization {
    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        latent_batch
            .payload
            .as_batch()
            .map_err(|err| EngineError::engine(err.to_string()))
    }
}
