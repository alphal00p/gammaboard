use super::{Batch, BatchResult, IngestScalar, Point};
use crate::core::{AccumulatorConfig, BuildError, EngineError, EvalError};
use crate::sampling::LatentBatch;
use crate::utils::domain::Domain;

#[derive(Debug, Clone, Copy)]
pub struct EvalBatchOptions {
    pub require_training_values: bool,
}

pub trait Evaluator: Send {
    fn get_domain(&self) -> Domain;
    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError>;
}

pub trait ScalarSampleEvaluator {
    fn eval_scalar_sample(&mut self, batch: &Batch, sample_idx: usize) -> Result<f64, EvalError>;

    fn eval_scalar_into<O: IngestScalar + ?Sized>(
        &mut self,
        batch: &Batch,
        accumulator: &mut O,
        require_training_values: bool,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let mut training_values = require_training_values.then(|| Vec::with_capacity(batch.size()));
        for (sample_idx, point) in batch.points().iter().enumerate() {
            let value = self.eval_scalar_sample(batch, sample_idx)?;
            accumulator.ingest_scalar(value, point);
            if let Some(values) = training_values.as_mut() {
                values.push(value * point.total_weight());
            }
        }
        Ok(training_values)
    }
}

pub trait ScalarValueEvaluator {
    fn ingest_scalar_values<O: IngestScalar + ?Sized>(
        &self,
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
}

impl<T> ScalarValueEvaluator for T {}

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

    fn eval_scalar_batch_into<O: IngestScalar + ?Sized>(
        &mut self,
        batch: &Batch,
        accumulator: &mut O,
        require_training_values: bool,
    ) -> Result<Option<Vec<f64>>, EvalError> {
        let values = self.eval_scalar_batch(batch)?;
        self.ingest_scalar_values(
            values.as_slice(),
            batch.points(),
            require_training_values,
            accumulator,
        )
    }
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
