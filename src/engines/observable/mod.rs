mod complex;
mod scalar;

use crate::engines::observable::complex::ComplexObservable;

use super::{BuildError, BuildFromJson, EngineError};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use strum::{AsRefStr, Display};

pub use self::scalar::ScalarObservable;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ObservableImplementation {
    Scalar,
    Complex,
}

/// Aggregates per-sample observables to batch-level and run-level snapshots.
#[enum_dispatch]
pub trait Observable: Send {
    fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError>;
    fn merge(&mut self, other: &ObservableEngine) -> Result<(), EngineError>;
    fn snapshot(&self) -> Result<JsonValue, EngineError>;
}

#[enum_dispatch(Observable)]
pub enum ObservableEngine {
    Scalar(ScalarObservable),
    Complex(ComplexObservable),
}

impl ObservableEngine {
    pub fn build(
        implementation: ObservableImplementation,
        params: &JsonValue,
    ) -> Result<Self, BuildError> {
        match implementation {
            ObservableImplementation::Scalar => {
                Ok(Self::Scalar(ScalarObservable::from_json(params)?))
            }
            ObservableImplementation::Complex => {
                Ok(Self::Complex(ComplexObservable::from_json(params)?))
            }
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Scalar(_) => "scalar",
            Self::Complex(_) => "complex",
        }
    }
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

    pub fn build(&self) -> Result<ObservableEngine, BuildError> {
        ObservableEngine::build(self.implementation, &self.params)
    }
}
