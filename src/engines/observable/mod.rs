mod scalar;

use super::{BuildError, BuildFromJson, EngineError};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;
use std::fmt;

use self::scalar::ScalarObservableAggregator;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservableImplementation {
    Scalar,
}

impl ObservableImplementation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
        }
    }
}

impl fmt::Display for ObservableImplementation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

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

/// Aggregates per-sample observables to batch-level and run-level snapshots.
#[enum_dispatch]
pub trait Observable: Send {
    fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError>;
    fn merge_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError>;
    fn snapshot(&self) -> Result<JsonValue, EngineError>;
}

#[enum_dispatch(Observable)]
pub enum ObservableEngine {
    Scalar(ScalarObservableAggregator),
}

impl ObservableEngine {
    pub fn build(
        implementation: ObservableImplementation,
        params: &JsonValue,
    ) -> Result<Self, BuildError> {
        match implementation {
            ObservableImplementation::Scalar => {
                Ok(Self::Scalar(ScalarObservableAggregator::from_json(params)?))
            }
        }
    }
}
