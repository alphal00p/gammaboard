use crate::core::EngineError;
use crate::evaluation::{Batch, Materializer};
use crate::sampling::LatentBatch;

pub struct IdentityMaterializer;

impl IdentityMaterializer {
    pub fn new() -> Self {
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
}
