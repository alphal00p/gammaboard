use crate::core::{BuildError, EngineError, EvalError, ObservableConfig};
use num::complex::Complex64;
use serde_json::{Value as JsonValue, json};

use super::{Batch, BatchResult, IngestComplex, IngestScalar, PointSpec};
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

pub trait ScalarSampleEvaluator {
    fn eval_scalar_sample(&mut self, batch: &Batch, sample_idx: usize) -> Result<f64, EvalError>;

    fn eval_scalar_into<O: IngestScalar>(
        &mut self,
        batch: &Batch,
        observable: &mut O,
        require_training_values: bool,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let mut training_values = require_training_values.then(|| Vec::with_capacity(batch.size()));
        for sample_idx in 0..batch.size() {
            let value = self.eval_scalar_sample(batch, sample_idx)?;
            let weight = weights[sample_idx];
            observable.ingest_scalar(value, weight);
            if let Some(values) = training_values.as_mut() {
                values.push(value * weight);
            }
        }
        Ok(training_values)
    }
}

pub trait ComplexSampleEvaluator {
    fn eval_complex_sample(
        &mut self,
        batch: &Batch,
        sample_idx: usize,
    ) -> Result<Complex64, EvalError>;

    fn eval_complex_into<O: IngestComplex>(
        &mut self,
        batch: &Batch,
        observable: &mut O,
        require_training_values: bool,
        training_projection: impl Fn(Complex64) -> f64,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let weights = batch
            .weights()
            .as_slice()
            .ok_or_else(|| EvalError::eval("Batch weights array must be standard-layout"))?;
        let mut training_values = require_training_values.then(|| Vec::with_capacity(batch.size()));
        for sample_idx in 0..batch.size() {
            let value = self.eval_complex_sample(batch, sample_idx)?;
            let weight = weights[sample_idx];
            observable.ingest_complex(value, weight);
            if let Some(values) = training_values.as_mut() {
                values.push(training_projection(value) * weight);
            }
        }
        Ok(training_values)
    }
}

pub trait Parametrization: Send + Sync {
    fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError>;

    fn snapshot(&self) -> Result<ParametrizationSnapshot, EngineError>;
}
