use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::any::Any;

use crate::{
    EngineError,
    engines::{BuildFromJson, observable::ComplexIngest, observable::ScalarIngest},
};

use super::Observable;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ComplexObservableParams {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexObservable {
    pub count: i64,
    pub real_sum: f64,
    pub imag_sum: f64,
    pub abs_sum: f64,
    pub abs_sq_sum: f64,
    pub real_sq_sum: f64,
    pub imag_sq_sum: f64,
    pub weight_sum: f64,
}

impl Observable for ComplexObservable {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &'static str {
        "complex"
    }

    fn as_complex_ingest(&mut self) -> Option<&mut dyn ComplexIngest> {
        Some(self)
    }

    fn as_scalar_ingest(&mut self) -> Option<&mut dyn ScalarIngest> {
        Some(self)
    }

    fn load_state_from_json(&mut self, state: &JsonValue) -> Result<(), EngineError> {
        *self = Self::deserialize(state).map_err(|err| EngineError::Build(err.to_string()))?;
        Ok(())
    }

    fn snapshot(&self) -> Result<JsonValue, EngineError> {
        serde_json::to_value(self).map_err(|err| EngineError::Build(err.to_string()))
    }

    fn merge(&mut self, other: &dyn Observable) -> Result<(), EngineError> {
        let Some(other) = other.as_any().downcast_ref::<ComplexObservable>() else {
            return Err(EngineError::Engine(format!(
                "cannot merge complex observable with {}",
                other.name()
            )));
        };

        self.count += other.count;
        self.real_sum += other.real_sum;
        self.imag_sum += other.imag_sum;
        self.abs_sum += other.abs_sum;
        self.abs_sq_sum += other.abs_sq_sum;
        self.real_sq_sum += other.real_sq_sum;
        self.imag_sq_sum += other.imag_sq_sum;
        self.weight_sum += other.weight_sum;
        Ok(())
    }
}

impl ComplexIngest for ComplexObservable {
    fn ingest_complex(&mut self, value: num::complex::Complex64, weight: f64) {
        self.add_sample(value, weight);
    }
}

impl ScalarIngest for ComplexObservable {
    fn ingest_scalar(&mut self, value: f64, weight: f64) {
        self.add_sample(num::complex::Complex64::new(value, 0.0), weight);
    }
}

impl BuildFromJson for ComplexObservable {
    type Params = ComplexObservableParams;

    fn from_parsed_params(_params: Self::Params) -> Result<Self, crate::BuildError> {
        Ok(Self {
            count: 0,
            real_sum: 0.0,
            imag_sum: 0.0,
            abs_sum: 0.0,
            abs_sq_sum: 0.0,
            real_sq_sum: 0.0,
            imag_sq_sum: 0.0,
            weight_sum: 0.0,
        })
    }
}

impl ComplexObservable {
    pub fn add_sample(&mut self, value: num::complex::Complex64, weight: f64) {
        self.count += 1;
        self.real_sum += value.re * weight;
        self.imag_sum += value.im * weight;
        self.abs_sum += value.norm() * weight;
        self.abs_sq_sum += value.norm().powi(2) * weight;
        self.real_sq_sum += value.re.powi(2) * weight;
        self.imag_sq_sum += value.im.powi(2) * weight;
        self.weight_sum += weight;
    }
}
