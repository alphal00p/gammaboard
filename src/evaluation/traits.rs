use super::{Batch, BatchResult, IngestScalar, Point};
use crate::core::{AccumulatorConfig, EngineError};
use crate::sampling::LatentBatch;
use crate::utils::domain::Domain;
use serde_json::{Value as JsonValue, json};

#[derive(Debug, Clone, Copy)]
pub struct EvalBatchOptions {
    pub require_training_values: bool,
}

pub trait Evaluator: Send {
    fn domain(&self) -> Domain;
    fn metadata(&self) -> JsonValue {
        json!({})
    }

    fn diagnostics(&mut self) -> JsonValue {
        json!({})
    }

    /// Unweighted, sample-major values used by runtime-owned evaluation modifiers.
    fn eval_values(
        &mut self,
        _batch: &Batch,
        _components: &[String],
    ) -> Result<Vec<f64>, EngineError> {
        Err(EngineError::invalid_input(
            "this evaluator does not expose per-sample values for stratification",
        ))
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EngineError>;
}

pub fn ingest_scalar_values<O: IngestScalar + ?Sized>(
    values: &[f64],
    points: &[Point],
    require_training_values: bool,
    accumulator: &mut O,
) -> Result<Option<Vec<f64>>, EngineError> {
    if values.len() != points.len() {
        return Err(EngineError::eval(format!(
            "cannot ingest scalar values: values length {} does not match points length {}",
            values.len(),
            points.len()
        )));
    }
    let mut training_values = require_training_values.then(|| Vec::with_capacity(values.len()));
    for (sample, (value, point)) in values.iter().zip(points.iter()).enumerate() {
        accumulator
            .ingest_scalar(*value, point)
            .map_err(|err| EngineError::eval(format!("sample {sample}: {err}")))?;
        if let Some(training_values) = training_values.as_mut() {
            training_values.push(*value * point.total_weight());
        }
    }
    Ok(training_values)
}

pub trait Materializer: Send {
    fn validate_domain(&self, _domain: &Domain) -> Result<(), EngineError> {
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError>;
}

pub trait BatchTransform: Send {
    fn validate_domain(&self, _domain: &Domain) -> Result<(), EngineError> {
        Ok(())
    }

    fn apply(&mut self, batch: Batch) -> Result<Batch, EngineError>;
}
