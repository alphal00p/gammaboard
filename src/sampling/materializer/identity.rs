use crate::core::EngineError;
use crate::evaluation::{Batch, Materializer};
use crate::sampling::LatentBatch;

pub struct IdentityMaterializer {
    fail_on_materialize_batch_nr: Option<usize>,
    materialized_batches_total: usize,
}

impl IdentityMaterializer {
    pub fn new() -> Self {
        Self {
            fail_on_materialize_batch_nr: None,
            materialized_batches_total: 0,
        }
    }

    pub fn new_with_failure(fail_on_materialize_batch_nr: Option<usize>) -> Self {
        Self {
            fail_on_materialize_batch_nr,
            materialized_batches_total: 0,
        }
    }
}

impl Materializer for IdentityMaterializer {
    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError> {
        self.materialized_batches_total = self.materialized_batches_total.saturating_add(1);
        if self
            .fail_on_materialize_batch_nr
            .is_some_and(|n| n > 0 && self.materialized_batches_total == n)
        {
            return Err(EngineError::engine(format!(
                "identity materializer injected failure on batch {}",
                self.materialized_batches_total
            )));
        }
        latent_batch
            .payload
            .as_batch()
            .map_err(|err| EngineError::engine(err.to_string()))
    }
}
