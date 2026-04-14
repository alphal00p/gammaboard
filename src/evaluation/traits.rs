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

fn dense_rectangular_inputs(
    batch: &Batch,
    discrete_dims: usize,
    continuous_dims: usize,
) -> Result<(Vec<i64>, Vec<f64>), EvalError> {
    let mut xs_discrete = Vec::with_capacity(batch.size().saturating_mul(discrete_dims));
    let mut xs_continuous = Vec::with_capacity(batch.size().saturating_mul(continuous_dims));
    for (sample_idx, point) in batch.points().iter().enumerate() {
        if point.discrete.len() != discrete_dims {
            return Err(EvalError::eval(format!(
                "homogeneous batch evaluator expected {discrete_dims} discrete dimensions, sample {sample_idx} has {}",
                point.discrete.len(),
            )));
        }
        if point.continuous.len() != continuous_dims {
            return Err(EvalError::eval(format!(
                "homogeneous batch evaluator expected {continuous_dims} continuous dimensions, sample {sample_idx} has {}",
                point.continuous.len(),
            )));
        }
        xs_discrete.extend_from_slice(&point.discrete);
        xs_continuous.extend_from_slice(&point.continuous);
    }
    Ok((xs_discrete, xs_continuous))
}

pub trait ScalarBatchEvaluator: ScalarValueEvaluator {
    fn discrete_dims(&self) -> usize;
    fn continuous_dims(&self) -> usize;
    fn eval_scalar_rectangular_batch(
        &mut self,
        xs_discrete_row_major: &[i64],
        xs_continuous_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<f64>, EvalError>;

    fn eval_scalar_batch(&mut self, batch: &Batch) -> Result<Vec<f64>, EvalError> {
        let nr_samples = batch.size();
        let (xs_discrete, xs_continuous) =
            dense_rectangular_inputs(batch, self.discrete_dims(), self.continuous_dims())?;
        let values =
            self.eval_scalar_rectangular_batch(&xs_discrete, &xs_continuous, nr_samples)?;
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
    fn discrete_dims(&self) -> usize;
    fn continuous_dims(&self) -> usize;
    fn eval_complex_rectangular_batch(
        &mut self,
        xs_discrete_row_major: &[i64],
        xs_continuous_row_major: &[f64],
        nr_samples: usize,
    ) -> Result<Vec<Complex64>, EvalError>;

    fn eval_complex_batch(&mut self, batch: &Batch) -> Result<Vec<Complex64>, EvalError> {
        let nr_samples = batch.size();
        let (xs_discrete, xs_continuous) =
            dense_rectangular_inputs(batch, self.discrete_dims(), self.continuous_dims())?;
        let values =
            self.eval_complex_rectangular_batch(&xs_discrete, &xs_continuous, nr_samples)?;
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
        discrete_dims: usize,
        continuous_dims: usize,
    }

    impl ScalarBatchEvaluator for EchoScalarBatch {
        fn discrete_dims(&self) -> usize {
            self.discrete_dims
        }

        fn continuous_dims(&self) -> usize {
            self.continuous_dims
        }

        fn eval_scalar_rectangular_batch(
            &mut self,
            xs_discrete_row_major: &[i64],
            xs_continuous_row_major: &[f64],
            nr_samples: usize,
        ) -> Result<Vec<f64>, EvalError> {
            let mut out = Vec::with_capacity(nr_samples);
            for sample_idx in 0..nr_samples {
                out.push(
                    xs_discrete_row_major
                        .get(sample_idx * self.discrete_dims)
                        .copied()
                        .unwrap_or_default() as f64
                        + xs_continuous_row_major[sample_idx * self.continuous_dims],
                );
            }
            Ok(out)
        }
    }

    #[test]
    fn scalar_batch_helper_rejects_wrong_discrete_dims() {
        let batch = Batch::from_points([Point::new(vec![1.0], vec![0], 1.0)]).expect("batch");
        let mut evaluator = EchoScalarBatch {
            discrete_dims: 0,
            continuous_dims: 1,
        };
        let err = evaluator
            .eval_scalar_batch(&batch)
            .expect_err("expected error");
        assert!(
            err.to_string().contains("expected 0 discrete dimensions"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn scalar_batch_helper_rejects_ragged_points() {
        let batch = Batch::from_points([Point::new(vec![1.0, 2.0], vec![], 1.0)]).expect("batch");
        let mut evaluator = EchoScalarBatch {
            discrete_dims: 0,
            continuous_dims: 1,
        };
        let err = evaluator
            .eval_scalar_batch(&batch)
            .expect_err("expected error");
        assert!(
            err.to_string().contains("expected 1 continuous dimensions"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn scalar_batch_helper_accepts_mixed_rectangular_points() {
        let batch = Batch::from_points([
            Point::new(vec![1.5], vec![2], 1.0),
            Point::new(vec![3.5], vec![4], 1.0),
        ])
        .expect("batch");
        let mut evaluator = EchoScalarBatch {
            discrete_dims: 1,
            continuous_dims: 1,
        };
        let values = evaluator.eval_scalar_batch(&batch).expect("values");
        assert_eq!(values, vec![3.5, 7.5]);
    }
}
