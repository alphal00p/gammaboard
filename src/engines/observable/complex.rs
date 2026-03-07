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
        let weight = weight.abs();
        let weighted_real = value.re * weight;
        let weighted_imag = value.im * weight;
        let weighted_abs = value.norm() * weight;
        self.count += 1;
        self.real_sum += weighted_real;
        self.imag_sum += weighted_imag;
        self.abs_sum += weighted_abs;
        self.abs_sq_sum += weighted_abs * weighted_abs;
        self.real_sq_sum += weighted_real * weighted_real;
        self.imag_sq_sum += weighted_imag * weighted_imag;
        self.weight_sum += weight;
    }
}

#[cfg(test)]
mod tests {
    use super::ComplexObservable;
    use num::complex::Complex64;

    #[test]
    fn add_sample_uses_weighted_contribution_moments() {
        let mut observable = ComplexObservable {
            count: 0,
            real_sum: 0.0,
            imag_sum: 0.0,
            abs_sum: 0.0,
            abs_sq_sum: 0.0,
            real_sq_sum: 0.0,
            imag_sq_sum: 0.0,
            weight_sum: 0.0,
        };

        observable.add_sample(Complex64::new(3.0, 4.0), 2.0);

        // Weighted contribution is (6 + 8i), with |.| = 10.
        assert_eq!(observable.count, 1);
        assert_eq!(observable.real_sum, 6.0);
        assert_eq!(observable.imag_sum, 8.0);
        assert_eq!(observable.abs_sum, 10.0);
        assert_eq!(observable.real_sq_sum, 36.0);
        assert_eq!(observable.imag_sq_sum, 64.0);
        assert_eq!(observable.abs_sq_sum, 100.0);
        assert_eq!(observable.weight_sum, 2.0);
    }

    #[test]
    fn add_sample_normalizes_negative_weights() {
        let mut observable = ComplexObservable {
            count: 0,
            real_sum: 0.0,
            imag_sum: 0.0,
            abs_sum: 0.0,
            abs_sq_sum: 0.0,
            real_sq_sum: 0.0,
            imag_sq_sum: 0.0,
            weight_sum: 0.0,
        };

        observable.add_sample(Complex64::new(1.5, -2.0), -3.0);

        assert_eq!(observable.real_sum, 4.5);
        assert_eq!(observable.imag_sum, -6.0);
        assert_eq!(observable.weight_sum, 3.0);
    }
}
