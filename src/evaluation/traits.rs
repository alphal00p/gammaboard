use super::{Batch, BatchResult, IngestScalar, Point};
use crate::core::{AccumulatorConfig, BuildError, EngineError, EvalError};
use crate::sampling::LatentBatch;
use crate::utils::domain::Domain;
use serde_json::{Value as JsonValue, json};

#[derive(Debug, Clone, Copy)]
pub struct EvalBatchOptions {
    pub require_training_values: bool,
}

pub trait Evaluator: Send {
    fn get_domain(&self) -> Domain;
    fn metadata(&self) -> JsonValue {
        json!({})
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError>;
}

pub fn ingest_scalar_values<O: IngestScalar + ?Sized>(
    values: &[f64],
    points: &[Point],
    require_training_values: bool,
    accumulator: &mut O,
) -> Result<Option<Vec<f64>>, EvalError> {
    if values.len() != points.len() {
        return Err(EvalError::eval(format!(
            "cannot ingest scalar values: values length {} does not match points length {}",
            values.len(),
            points.len()
        )));
    }
    let mut training_values = require_training_values.then(|| Vec::with_capacity(values.len()));
    for (value, point) in values.iter().zip(points.iter()) {
        accumulator.ingest_scalar(*value, point);
        if let Some(training_values) = training_values.as_mut() {
            training_values.push(*value * point.total_weight());
        }
    }
    Ok(training_values)
}

pub trait Materializer: Send {
    fn validate_domain(&self, _domain: &Domain) -> Result<(), BuildError> {
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError>;
}

pub trait BatchTransform: Send + Sync {
    fn validate_domain(&self, _domain: &Domain) -> Result<(), BuildError> {
        Ok(())
    }

    fn apply(&self, batch: Batch) -> Result<Batch, EngineError>;
}
