mod complex;
mod scalar;

use crate::engines::observable::complex::ComplexObservable;

use super::{BuildError, BuildFromJson, EngineError};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::any::Any;
use strum::{AsRefStr, Display};

pub use self::scalar::ScalarObservable;

pub trait ScalarIngest: Send {
    fn ingest_scalar(&mut self, value: f64, weight: f64);
}

pub trait ComplexIngest: Send {
    fn ingest_complex(&mut self, value: num::complex::Complex64, weight: f64);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ObservableImplementation {
    Scalar,
    Complex,
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

#[derive(Debug, Clone)]
pub struct ObservableFactory {
    pub(crate) implementation: ObservableImplementation,
    params: JsonValue,
}

impl ObservableFactory {
    pub fn new(implementation: ObservableImplementation, params: JsonValue) -> Self {
        Self {
            implementation,
            params,
        }
    }

    pub fn build(&self) -> Result<Box<dyn Observable>, BuildError> {
        match self.implementation {
            ObservableImplementation::Scalar => {
                Ok(Box::new(ScalarObservable::from_json(&self.params)?))
            }
            ObservableImplementation::Complex => {
                Ok(Box::new(ComplexObservable::from_json(&self.params)?))
            }
        }
    }
}
