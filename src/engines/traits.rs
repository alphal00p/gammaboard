//! Runtime engine traits for evaluator, sampler-aggregator, and observable aggregation.

use super::{BuildError, EngineError, EngineState, EvalError};
use crate::batch::{Batch, EvaluatedSample, PointSpec, PointView};
use serde_json::Value as JsonValue;

/// Evaluates integrand values for sample points.
pub trait Evaluator: Send + Sync {
    fn from_params(params: &JsonValue) -> Result<Self, BuildError>
    where
        Self: Sized;

    fn implementation(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;

    fn eval_point(&self, point: PointView<'_>) -> Result<EvaluatedSample, EvalError>;

    fn eval_batch(&self, batch: &Batch) -> Result<Vec<EvaluatedSample>, EvalError> {
        let mut samples = Vec::with_capacity(batch.size());
        for point in batch.iter_points() {
            samples.push(self.eval_point(point)?);
        }
        Ok(samples)
    }
}

impl<T> Evaluator for Box<T>
where
    T: Evaluator + ?Sized,
{
    fn from_params(_params: &JsonValue) -> Result<Self, BuildError>
    where
        Self: Sized,
    {
        Err(BuildError::build(
            "Box<T>::from_params is not supported; build concrete evaluator type",
        ))
    }

    fn eval_point(&self, point: PointView<'_>) -> Result<EvaluatedSample, EvalError> {
        (**self).eval_point(point)
    }

    fn implementation(&self) -> &'static str {
        (**self).implementation()
    }

    fn version(&self) -> &'static str {
        (**self).version()
    }

    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        (**self).validate_point_spec(point_spec)
    }

    fn eval_batch(&self, batch: &Batch) -> Result<Vec<EvaluatedSample>, EvalError> {
        (**self).eval_batch(batch)
    }
}

/// Owns adaptive sampling training for a single run.
pub trait SamplerAggregatorEngine: Send {
    fn from_params(params: &JsonValue) -> Result<Self, BuildError>
    where
        Self: Sized;

    fn implementation(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn init(&mut self, state: Option<EngineState>) -> Result<(), EngineError>;
    fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<Batch>, EngineError>;
    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError>;
}

impl<T> SamplerAggregatorEngine for Box<T>
where
    T: SamplerAggregatorEngine + ?Sized,
{
    fn from_params(_params: &JsonValue) -> Result<Self, BuildError>
    where
        Self: Sized,
    {
        Err(BuildError::build(
            "Box<T>::from_params is not supported; build concrete sampler-aggregator type",
        ))
    }

    fn implementation(&self) -> &'static str {
        (**self).implementation()
    }

    fn version(&self) -> &'static str {
        (**self).version()
    }

    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError> {
        (**self).validate_point_spec(point_spec)
    }

    fn init(&mut self, state: Option<EngineState>) -> Result<(), EngineError> {
        (**self).init(state)
    }

    fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<Batch>, EngineError> {
        (**self).produce_batches(max_batches)
    }

    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError> {
        (**self).ingest_training_weights(training_weights)
    }
}

/// Aggregates per-sample observables to batch-level and run-level snapshots.
pub trait AggregatedObservable: Send {
    fn from_params(params: &JsonValue) -> Result<Self, BuildError>
    where
        Self: Sized;

    fn implementation(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn restore(&mut self, snapshot: Option<JsonValue>) -> Result<(), EngineError>;
    fn ingest_sample_observable(
        &mut self,
        sample_observable: &JsonValue,
    ) -> Result<(), EngineError>;
    fn ingest_batch_observable(&mut self, batch_observable: &JsonValue) -> Result<(), EngineError>;
    fn snapshot(&self) -> Result<JsonValue, EngineError>;
}

impl<T> AggregatedObservable for Box<T>
where
    T: AggregatedObservable + ?Sized,
{
    fn from_params(_params: &JsonValue) -> Result<Self, BuildError>
    where
        Self: Sized,
    {
        Err(BuildError::build(
            "Box<T>::from_params is not supported; build concrete observable type",
        ))
    }

    fn implementation(&self) -> &'static str {
        (**self).implementation()
    }

    fn version(&self) -> &'static str {
        (**self).version()
    }

    fn restore(&mut self, snapshot: Option<JsonValue>) -> Result<(), EngineError> {
        (**self).restore(snapshot)
    }

    fn ingest_sample_observable(
        &mut self,
        sample_observable: &JsonValue,
    ) -> Result<(), EngineError> {
        (**self).ingest_sample_observable(sample_observable)
    }

    fn ingest_batch_observable(&mut self, batch_observable: &JsonValue) -> Result<(), EngineError> {
        (**self).ingest_batch_observable(batch_observable)
    }

    fn snapshot(&self) -> Result<JsonValue, EngineError> {
        (**self).snapshot()
    }
}
