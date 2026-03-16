mod complex;
mod scalar;

use crate::core::EngineError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;

pub use self::complex::ComplexObservableState;
pub use self::scalar::ScalarObservableState;

pub trait Observable: Clone + Serialize + DeserializeOwned {
    type Persistent: Clone + Serialize + DeserializeOwned;

    fn merge(&mut self, other: Self);
    fn get_persistent(&self) -> Self::Persistent;

    fn to_persistent_json(&self) -> Result<JsonValue, EngineError> {
        serde_json::to_value(self.get_persistent()).map_err(|err| {
            EngineError::build(format!(
                "failed to serialize persistent observable payload: {err}"
            ))
        })
    }
}

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
                Observable::merge(left, right);
                Ok(())
            }
            (Self::Complex(left), Self::Complex(right)) => {
                Observable::merge(left, right);
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

    pub fn to_persistent_json(&self) -> Result<JsonValue, EngineError> {
        match self {
            Self::Scalar(observable) => observable.to_persistent_json(),
            Self::Complex(observable) => observable.to_persistent_json(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ObservableState, ScalarObservableState};

    #[test]
    fn persistent_json_roundtrips_without_enum_tag() {
        let snapshot = ObservableState::Scalar(ScalarObservableState {
            count: 2,
            sum_weighted_value: 3.0,
            sum_abs: 4.0,
            sum_sq: 5.0,
        })
        .to_persistent_json()
        .expect("persistent snapshot");

        assert_eq!(snapshot.get("kind"), None);
    }
}
