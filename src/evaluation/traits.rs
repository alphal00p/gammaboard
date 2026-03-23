use crate::core::{BuildError, EngineError, EvalError, ObservableConfig};
use num::complex::Complex64;
use serde_json::{Value as JsonValue, json};

use super::{Batch, BatchResult, IngestComplex, IngestScalar, PointSpec};
use crate::sampling::LatentBatch;

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

pub trait ScalarValueEvaluator {
    fn ingest_scalar_values<O: IngestScalar>(
        &self,
        values: &[f64],
        weights: &[f64],
        capture_training_values: bool,
        observable: &mut O,
    ) -> Option<Vec<f64>> {
        let mut training_values = capture_training_values.then(|| Vec::with_capacity(values.len()));
        for (sample_idx, value) in values.iter().enumerate() {
            observable.ingest_scalar(*value, weights[sample_idx]);
            if let Some(training_values) = training_values.as_mut() {
                training_values.push(*value * weights[sample_idx]);
            }
        }
        training_values
    }
}

impl<T> ScalarValueEvaluator for T {}

pub trait ComplexValueEvaluator {
    fn ingest_complex_values<O: IngestComplex>(
        &self,
        values: &[Complex64],
        weights: &[f64],
        capture_training_values: bool,
        observable: &mut O,
        training_projection: impl Fn(Complex64) -> f64,
    ) -> Option<Vec<f64>> {
        let mut training_values = capture_training_values.then(|| Vec::with_capacity(values.len()));
        for (sample_idx, value) in values.iter().enumerate() {
            observable.ingest_complex(*value, weights[sample_idx]);
            if let Some(training_values) = training_values.as_mut() {
                training_values.push(training_projection(*value) * weights[sample_idx]);
            }
        }
        training_values
    }
}

impl<T> ComplexValueEvaluator for T {}

pub trait Materializer: Send + Sync {
    fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError>;
}

pub trait BatchTransform: Send + Sync {
    fn validate_point_spec(&self, _point_spec: &PointSpec) -> Result<(), BuildError> {
        Ok(())
    }

    fn apply(&self, batch: Batch) -> Result<Batch, EngineError>;
}
