use crate::engines::{BuildError, BuildFromJson, EngineError, Observable, ObservableEngine};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ScalarObservableParams {}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScalarObservable {
    pub count: i64,
    pub sum_weight: f64,
    pub sum_abs: f64,
    pub sum_sq: f64,
}

impl ScalarObservable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_sample(&mut self, value: f64, weight: f64) {
        let weight = weight.abs();
        self.count += 1;
        self.sum_weight += value * weight;
        self.sum_abs += value.abs();
        self.sum_sq += value * value;
    }
}

impl BuildFromJson for ScalarObservable {
    type Params = ScalarObservableParams;
    fn from_parsed_params(_params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new())
    }
}

impl Observable for ScalarObservable {
    fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
        *self = serde_json::from_value(state.clone()).map_err(|err| {
            EngineError::build(format!("invalid scalar observable payload: {err}"))
        })?;
        Ok(())
    }

    fn snapshot(&self) -> Result<JsonValue, EngineError> {
        serde_json::to_value(self).map_err(|err| {
            EngineError::build(format!("failed to serialize scalar observable: {err}"))
        })
    }

    fn merge(&mut self, other: &ObservableEngine) -> Result<(), EngineError> {
        let other = match other {
            ObservableEngine::Scalar(other) => other,
            _ => return Err(EngineError::build("invalid observable type")),
        };
        self.count += other.count;
        self.sum_weight += other.sum_weight;
        self.sum_abs += other.sum_abs;
        self.sum_sq += other.sum_sq;
        Ok(())
    }
}
