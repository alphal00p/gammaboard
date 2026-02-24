use crate::engines::{
    BuildError, BuildFromJson, EngineError, Observable, decode_observable_state,
    encode_observable_state,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ScalarObservableParams {}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ScalarState {
    count: i64,
    sum_weight: f64,
    sum_abs: f64,
    sum_sq: f64,
}

impl ScalarState {
    fn merge_from(&mut self, other: &Self) {
        self.count += other.count;
        self.sum_weight += other.sum_weight;
        self.sum_abs += other.sum_abs;
        self.sum_sq += other.sum_sq;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScalarSnapshot {
    count: i64,
    sum_weight: f64,
    sum_abs: f64,
    sum_sq: f64,
    #[serde(default)]
    mean: Option<f64>,
}

impl From<&ScalarState> for ScalarSnapshot {
    fn from(state: &ScalarState) -> Self {
        let mean = if state.count > 0 {
            Some(state.sum_weight / state.count as f64)
        } else {
            None
        };
        Self {
            count: state.count,
            sum_weight: state.sum_weight,
            sum_abs: state.sum_abs,
            sum_sq: state.sum_sq,
            mean,
        }
    }
}

impl From<ScalarSnapshot> for ScalarState {
    fn from(snapshot: ScalarSnapshot) -> Self {
        Self {
            count: snapshot.count,
            sum_weight: snapshot.sum_weight,
            sum_abs: snapshot.sum_abs,
            sum_sq: snapshot.sum_sq,
        }
    }
}

pub struct ScalarObservableAggregator {
    state: ScalarState,
}

impl ScalarObservableAggregator {
    pub fn new() -> Self {
        Self {
            state: ScalarState::default(),
        }
    }

    pub fn add_sample(&mut self, value: f64, weight: f64) {
        let weight = weight.abs();
        self.state.count += 1;
        self.state.sum_weight += value * weight;
        self.state.sum_abs += value.abs();
        self.state.sum_sq += value * value;
    }
}

impl BuildFromJson for ScalarObservableAggregator {
    type Params = ScalarObservableParams;
    const PARAMS_CONTEXT: &'static str = "scalar observable params";

    fn from_parsed_params(_params: Self::Params) -> Result<Self, BuildError> {
        Ok(Self::new())
    }
}

impl Observable for ScalarObservableAggregator {
    fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
        let decoded: ScalarSnapshot = decode_observable_state(state, "scalar observable")?;
        self.state = decoded.into();
        Ok(())
    }

    fn merge_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
        let decoded: ScalarSnapshot = decode_observable_state(state, "batch scalar observable")?;
        let other: ScalarState = decoded.into();
        self.state.merge_from(&other);
        Ok(())
    }

    fn snapshot(&self) -> Result<JsonValue, EngineError> {
        encode_observable_state(&ScalarSnapshot::from(&self.state), "scalar observable")
    }
}
