use crate::core::{BuildError, EngineError, EvalError, ObservableConfig};
use serde_json::{Value as JsonValue, json};

use super::{Batch, BatchResult, PointSpec};
use crate::sampling::{LatentBatch, ParametrizationSnapshot};

#[derive(Debug, Clone, Copy)]
pub struct EvalBatchOptions {
    pub require_training_values: bool,
}

pub trait Evaluator: Send {
    fn get_point_spec(&self) -> PointSpec;
    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError>;
    fn get_init_metadata(&self) -> JsonValue {
        json!({})
    }
}

pub trait Parametrization: Send + Sync {
    fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError>;

    fn snapshot(&self) -> Result<ParametrizationSnapshot, EngineError>;
}
