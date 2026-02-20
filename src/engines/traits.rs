//! Runtime engine traits for evaluator, sampler-aggregator, and observable aggregation.

use super::{BuildError, EngineError, EvalError};
use crate::batch::{Batch, BatchResult, PointSpec};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;

pub fn decode_observable_state<T>(value: &JsonValue, context: &str) -> Result<T, EngineError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value.clone())
        .map_err(|err| EngineError::engine(format!("invalid {context} payload: {err}")))
}

pub fn encode_observable_state<T>(state: &T, context: &str) -> Result<JsonValue, EngineError>
where
    T: Serialize,
{
    serde_json::to_value(state)
        .map_err(|err| EngineError::engine(format!("failed to serialize {context}: {err}")))
}

/// Evaluates integrand values for sample points.
pub trait Evaluator: Send + Sync {
    fn implementation(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn eval_batch(
        &self,
        batch: &Batch,
        observable: &mut dyn Observable,
    ) -> Result<BatchResult, EvalError>;
}

/// Owns adaptive sampling training for a single run.
pub trait SamplerAggregatorEngine: Send {
    fn implementation(&self) -> &'static str;
    fn version(&self) -> &'static str;
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn init(&mut self) -> Result<(), EngineError>;
    fn produce_batches(&mut self, max_batches: usize) -> Result<Vec<Batch>, EngineError>;
    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError>;
}

/// Aggregates per-sample observables to batch-level and run-level snapshots.
pub trait Observable: Send {
    fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError>;
    fn merge_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError>;
    fn snapshot(&self) -> Result<JsonValue, EngineError>;
}
