use crate::core::{BuildError, EngineError, EvalError, ObservableConfig};
use crate::utils::domain::Domain;
use num::complex::Complex64;

use super::{Batch, BatchResult, IngestComplex, IngestScalar};
use crate::sampling::LatentBatch;

#[derive(Debug, Clone, Copy)]
pub struct EvalBatchOptions {
    pub require_training_values: bool,
}

pub trait Evaluator: Send {
    fn get_domain(&self) -> Domain;
    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable: &ObservableConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError>;
}

pub trait ScalarSampleEvaluator {
    fn eval_scalar_sample(&mut self, batch: &Batch, sample_idx: usize) -> Result<f64, EvalError>;

    fn eval_scalar_into<O: IngestScalar>(
        &mut self,
        batch: &Batch,
        observable: &mut O,
        require_training_values: bool,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let mut training_values = require_training_values.then(|| Vec::with_capacity(batch.size()));
        for sample_idx in 0..batch.size() {
            let value = self.eval_scalar_sample(batch, sample_idx)?;
            let weight = batch
                .point(sample_idx)
                .map(|point| point.weight)
                .ok_or_else(|| EvalError::eval(format!("batch is missing sample {sample_idx}")))?;
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
        let mut training_values = require_training_values.then(|| Vec::with_capacity(batch.size()));
        for sample_idx in 0..batch.size() {
            let value = self.eval_complex_sample(batch, sample_idx)?;
            let weight = batch
                .point(sample_idx)
                .map(|point| point.weight)
                .ok_or_else(|| EvalError::eval(format!("batch is missing sample {sample_idx}")))?;
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
        require_training_values: bool,
        observable: &mut O,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        if values.len() != weights.len() {
            return Err(EvalError::eval(format!(
                "cannot ingest scalar values: values length {} does not match weights length {}",
                values.len(),
                weights.len()
            )));
        }
        let mut training_values = require_training_values.then(|| Vec::with_capacity(values.len()));
        for (sample_idx, value) in values.iter().enumerate() {
            observable.ingest_scalar(*value, weights[sample_idx]);
            if let Some(training_values) = training_values.as_mut() {
                training_values.push(*value * weights[sample_idx]);
            }
        }
        Ok(training_values)
    }
}

impl<T> ScalarValueEvaluator for T {}

pub trait ComplexValueEvaluator {
    fn ingest_complex_values<O: IngestComplex>(
        &self,
        values: &[Complex64],
        weights: &[f64],
        require_training_values: bool,
        observable: &mut O,
        training_projection: impl Fn(Complex64) -> f64,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        if values.len() != weights.len() {
            return Err(EvalError::eval(format!(
                "cannot ingest complex values: values length {} does not match weights length {}",
                values.len(),
                weights.len()
            )));
        }
        let mut training_values = require_training_values.then(|| Vec::with_capacity(values.len()));
        for (sample_idx, value) in values.iter().enumerate() {
            observable.ingest_complex(*value, weights[sample_idx]);
            if let Some(training_values) = training_values.as_mut() {
                training_values.push(training_projection(*value) * weights[sample_idx]);
            }
        }
        Ok(training_values)
    }
}

fn dense_continuous_inputs(batch: &Batch, input_dim: usize) -> Result<Vec<f64>, EvalError> {
    let mut xs = Vec::with_capacity(batch.size().saturating_mul(input_dim));
    for (sample_idx, point) in batch.points().iter().enumerate() {
        if !point.discrete.is_empty() {
            return Err(EvalError::eval(format!(
                "homogeneous batch evaluator expects only continuous inputs; sample {sample_idx} has {} discrete coordinates",
                point.discrete.len()
            )));
        }
        if point.continuous.len() != input_dim {
            return Err(EvalError::eval(format!(
                "homogeneous batch evaluator expected {input_dim} continuous dimensions, sample {sample_idx} has {}",
                point.continuous.len()
            )));
        }
        xs.extend_from_slice(&point.continuous);
    }
    Ok(xs)
}

pub trait ScalarBatchEvaluator: ScalarValueEvaluator {
    fn input_dim(&self) -> usize;
    fn eval_scalar_dense_batch(
        &mut self,
        xs_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError>;

    fn eval_scalar_batch(&mut self, batch: &Batch) -> Result<Vec<f64>, EvalError> {
        let nr_samples = batch.size();
        let values = self.eval_scalar_dense_batch(
            &dense_continuous_inputs(batch, self.input_dim())?,
            nr_samples,
        )?;
        if values.len() != nr_samples {
            return Err(EvalError::eval(format!(
                "scalar batch evaluator produced {} values for {} samples",
                values.len(),
                nr_samples
            )));
        }
        Ok(values)
    }

    fn eval_scalar_batch_into<O: IngestScalar>(
        &mut self,
        batch: &Batch,
        observable: &mut O,
        require_training_values: bool,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let values = self.eval_scalar_batch(batch)?;
        self.ingest_scalar_values(
            values.as_slice(),
            batch.weights().as_slice(),
            require_training_values,
            observable,
        )
    }
}

pub trait ComplexBatchEvaluator: ComplexValueEvaluator {
    fn input_dim(&self) -> usize;
    fn eval_complex_dense_batch(
        &mut self,
        xs_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<Complex64>, EvalError>;

    fn eval_complex_batch(&mut self, batch: &Batch) -> Result<Vec<Complex64>, EvalError> {
        let nr_samples = batch.size();
        let values = self.eval_complex_dense_batch(
            &dense_continuous_inputs(batch, self.input_dim())?,
            nr_samples,
        )?;
        if values.len() != nr_samples {
            return Err(EvalError::eval(format!(
                "complex batch evaluator produced {} values for {} samples",
                values.len(),
                nr_samples
            )));
        }
        Ok(values)
    }

    fn eval_complex_batch_into<O: IngestComplex>(
        &mut self,
        batch: &Batch,
        observable: &mut O,
        require_training_values: bool,
        training_projection: impl Fn(Complex64) -> f64,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let values = self.eval_complex_batch(batch)?;
        self.ingest_complex_values(
            values.as_slice(),
            batch.weights().as_slice(),
            require_training_values,
            observable,
            training_projection,
        )
    }
}

impl<T> ComplexValueEvaluator for T {}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::Point;

    struct EchoScalarBatch {
        input_dim: usize,
    }

    impl ScalarBatchEvaluator for EchoScalarBatch {
        fn input_dim(&self) -> usize {
            self.input_dim
        }

        fn eval_scalar_dense_batch(
            &mut self,
            xs_row_major: &[f64],
            nr_samples: usize,
        ) -> Result<Vec<f64>, EvalError> {
            let mut out = Vec::with_capacity(nr_samples);
            for sample_idx in 0..nr_samples {
                out.push(xs_row_major[sample_idx * self.input_dim]);
            }
            Ok(out)
        }
    }

    #[test]
    fn scalar_batch_helper_rejects_discrete_points() {
        let batch = Batch::from_points([Point::new(vec![1.0], vec![0], 1.0)]).expect("batch");
        let mut evaluator = EchoScalarBatch { input_dim: 1 };
        let err = evaluator
            .eval_scalar_batch(&batch)
            .expect_err("expected error");
        assert!(
            err.to_string().contains("continuous inputs"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn scalar_batch_helper_rejects_ragged_points() {
        let batch = Batch::from_points([Point::new(vec![1.0, 2.0], vec![], 1.0)]).expect("batch");
        let mut evaluator = EchoScalarBatch { input_dim: 1 };
        let err = evaluator
            .eval_scalar_batch(&batch)
            .expect_err("expected error");
        assert!(
            err.to_string().contains("expected 1 continuous dimensions"),
            "unexpected error: {err}"
        );
    }
}
