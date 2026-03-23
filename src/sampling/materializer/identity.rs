use super::MaterializerSnapshot;
use crate::core::EngineError;
use crate::evaluation::{Batch, Materializer};
use crate::sampling::LatentBatch;
use serde::{Deserialize, Serialize};

pub struct IdentityMaterializer;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct IdentityMaterializerParams {}

impl IdentityMaterializer {
    pub fn from_params(_params: IdentityMaterializerParams) -> Self {
        Self
    }
}

impl Materializer for IdentityMaterializer {
    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        latent_batch
            .payload
            .as_batch()
            .map_err(|err| EngineError::engine(err.to_string()))
    }

    fn snapshot(&self) -> Result<MaterializerSnapshot, EngineError> {
        Ok(MaterializerSnapshot::Identity {})
    }
}
