use crate::engines::{
    BuildError, BuildFromJson, EngineError, Observable, observable::ScalarIngest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::any::Any;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ScalarObservableParams {}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScalarObservable {
    pub count: i64,
    pub sum_weighted_value: f64,
    pub sum_abs: f64,
    pub sum_sq: f64,
}

impl ScalarObservable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_sample(&mut self, value: f64, weight: f64) {
        let weight = weight.abs();
        let weighted_value = value * weight;
        self.count += 1;
        self.sum_weighted_value += weighted_value;
        self.sum_abs += weighted_value.abs();
        // Keep second moment consistent with the actual Monte Carlo contribution.
        self.sum_sq += weighted_value * weighted_value;
    }
}

impl BuildFromJson for ScalarObservable {
    type Params = ScalarObservableParams;
    fn from_parsed_params(_params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new())
    }
}

impl Observable for ScalarObservable {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &'static str {
        "scalar"
    }

    fn as_scalar_ingest(&mut self) -> Option<&mut dyn ScalarIngest> {
        Some(self)
    }

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

    fn merge(&mut self, other: &dyn Observable) -> Result<(), EngineError> {
        let Some(other) = other.as_any().downcast_ref::<ScalarObservable>() else {
            return Err(EngineError::build(format!(
                "cannot merge scalar observable with {}",
                other.name()
            )));
        };
        self.count += other.count;
        self.sum_weighted_value += other.sum_weighted_value;
        self.sum_abs += other.sum_abs;
        self.sum_sq += other.sum_sq;
        Ok(())
    }
}

impl ScalarIngest for ScalarObservable {
    fn ingest_scalar(&mut self, value: f64, weight: f64) {
        self.add_sample(value, weight);
    }
}
