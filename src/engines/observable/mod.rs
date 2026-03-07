mod complex;
mod scalar;

use crate::engines::observable::complex::ComplexObservable;

use super::{BuildError, BuildFromJson, EngineError};
use serde_json::Value as JsonValue;
use std::any::Any;

pub use self::scalar::ScalarObservable;

pub trait ScalarIngest: Send {
    fn ingest_scalar(&mut self, value: f64, weight: f64);
}

pub trait ComplexIngest: Send {
    fn ingest_complex(&mut self, value: num::complex::Complex64, weight: f64);
}

/// Aggregates per-sample observables to batch-level and run-level snapshots.
pub trait Observable: Send {
    fn as_any(&self) -> &dyn Any;

    fn name(&self) -> &'static str;

    fn as_scalar_ingest(&mut self) -> Option<&mut dyn ScalarIngest> {
        None
    }

    fn as_complex_ingest(&mut self) -> Option<&mut dyn ComplexIngest> {
        None
    }

    fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError>;
    fn merge(&mut self, other: &dyn Observable) -> Result<(), EngineError>;
    fn snapshot(&self) -> Result<JsonValue, EngineError>;
}

impl crate::engines::ObservableConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Scalar { .. } => "scalar",
            Self::Complex { .. } => "complex",
        }
    }

    pub fn build(&self) -> Result<Box<dyn Observable>, BuildError> {
        match self {
            Self::Scalar { params } => Ok(Box::new(ScalarObservable::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
            Self::Complex { params } => Ok(Box::new(ComplexObservable::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
        }
    }
}
