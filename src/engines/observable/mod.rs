mod complex;
mod scalar;

use crate::engines::EngineError;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub use self::complex::ComplexObservableState;
pub use self::scalar::ScalarObservableState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservableState {
    Scalar(ScalarObservableState),
    Complex(ComplexObservableState),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SemanticObservableKind {
    #[default]
    Scalar,
    Complex,
}

impl SemanticObservableKind {
    pub fn empty_state(self) -> ObservableState {
        match self {
            Self::Scalar => ObservableState::empty_scalar(),
            Self::Complex => ObservableState::empty_complex(),
        }
    }
}

impl ObservableState {
    pub fn empty_scalar() -> Self {
        Self::Scalar(ScalarObservableState::default())
    }

    pub fn empty_complex() -> Self {
        Self::Complex(ComplexObservableState::default())
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Scalar(_) => "scalar",
            Self::Complex(_) => "complex",
        }
    }

    pub fn merge(&mut self, other: Self) -> Result<(), EngineError> {
        match (self, other) {
            (Self::Scalar(left), Self::Scalar(right)) => {
                left.merge(right);
                Ok(())
            }
            (Self::Complex(left), Self::Complex(right)) => {
                left.merge(right);
                Ok(())
            }
            (left, right) => Err(EngineError::engine(format!(
                "cannot merge {} observable with {} observable",
                left.kind_str(),
                right.kind_str(),
            ))),
        }
    }

    pub fn to_json(&self) -> Result<JsonValue, EngineError> {
        serde_json::to_value(self)
            .map_err(|err| EngineError::build(format!("failed to serialize observable: {err}")))
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, EngineError> {
        serde_json::from_value(value.clone())
            .map_err(|err| EngineError::build(format!("invalid observable payload: {err}")))
    }
}
