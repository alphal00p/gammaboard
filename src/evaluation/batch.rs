//! Batch abstraction for concrete evaluator-side materialized work.

use bincode::config::{Configuration, standard};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{error::Error, fmt};

use crate::evaluation::AccumulatorState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightFactor {
    pub label: String,
    pub value: f64,
}

impl WeightFactor {
    pub fn new(label: impl Into<String>, value: f64) -> Self {
        Self {
            label: label.into(),
            value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub continuous: Vec<f64>,
    pub discrete: Vec<i64>,
    pub weight_factors: Vec<WeightFactor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrand_value_re: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrand_value_im: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameterization_jacobian: Option<f64>,
}

impl Point {
    pub fn new(continuous: Vec<f64>, discrete: Vec<i64>, sampler_weight: f64) -> Self {
        Self {
            continuous,
            discrete,
            weight_factors: vec![WeightFactor::new("sampler_weight", sampler_weight)],
            integrand_value_re: None,
            integrand_value_im: None,
            parameterization_jacobian: None,
        }
    }

    pub fn total_weight(&self) -> f64 {
        self.weight_factors
            .iter()
            .map(|factor| factor.value)
            .product()
    }

    pub fn add_weight_factor(&mut self, label: impl Into<String>, value: f64) {
        self.weight_factors.push(WeightFactor::new(label, value));
    }

    pub fn factor_value(&self, label: &str) -> Option<f64> {
        self.weight_factors
            .iter()
            .find(|factor| factor.label == label)
            .map(|factor| factor.value)
    }

    pub fn factor_product_matching(&self, mut predicate: impl FnMut(&str) -> bool) -> Option<f64> {
        let mut product = 1.0_f64;
        let mut matched = false;
        for factor in &self.weight_factors {
            if predicate(factor.label.as_str()) {
                product *= factor.value;
                matched = true;
            }
        }
        matched.then_some(product)
    }

    pub fn clone_with_continuous_and_added_factor(
        &self,
        continuous: Vec<f64>,
        label: impl Into<String>,
        value: f64,
    ) -> Self {
        let mut next = self.clone();
        next.continuous = continuous;
        next.add_weight_factor(label, value);
        next
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchError {
    message: String,
}

impl BatchError {
    pub fn layout(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for BatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for BatchError {}

impl From<serde_json::Error> for BatchError {
    fn from(value: serde_json::Error) -> Self {
        Self::layout(format!("invalid batch json: {value}"))
    }
}

/// Concrete batch representation as a list of points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    points: Vec<Point>,
}

impl Batch {
    pub fn new(points: Vec<Point>) -> Result<Self, BatchError> {
        Ok(Self { points })
    }

    pub fn from_points(points: impl IntoIterator<Item = Point>) -> Result<Self, BatchError> {
        Self::new(points.into_iter().collect())
    }

    pub fn size(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn points(&self) -> &[Point] {
        &self.points
    }

    pub fn point(&self, sample_idx: usize) -> Option<&Point> {
        self.points.get(sample_idx)
    }

    pub fn weights(&self) -> Vec<f64> {
        self.points.iter().map(Point::total_weight).collect()
    }

    pub fn to_json(&self) -> JsonValue {
        serde_json::to_value(self).expect("Batch serialization should never fail")
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, BatchError> {
        Ok(serde_json::from_value(value.clone())?)
    }
}

/// Evaluator output for one batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub values: Option<Vec<f64>>,
    pub accumulator: AccumulatorState,
}

impl BatchResult {
    fn binary_config() -> Configuration {
        standard()
    }

    pub fn new(values: Option<Vec<f64>>, accumulator: AccumulatorState) -> Self {
        Self {
            values,
            accumulator,
        }
    }

    pub fn len(&self) -> usize {
        self.values.as_ref().map_or(0, Vec::len)
    }

    pub fn is_empty(&self) -> bool {
        self.values.as_ref().is_none_or(Vec::is_empty)
    }

    pub fn matches_batch(&self, batch: &Batch) -> bool {
        self.values
            .as_ref()
            .is_none_or(|values| values.len() == batch.size())
    }

    pub fn values_to_json(&self) -> JsonValue {
        match &self.values {
            Some(values) => {
                serde_json::to_value(values).expect("batch values serialization should never fail")
            }
            None => JsonValue::Null,
        }
    }

    pub fn values_to_bytes(&self) -> Result<Option<Vec<u8>>, BatchError> {
        self.values
            .as_ref()
            .map(|values| {
                bincode::serde::encode_to_vec(values, Self::binary_config()).map_err(|err| {
                    BatchError::layout(format!("invalid batch values payload: {err}"))
                })
            })
            .transpose()
    }

    pub fn validate_json_safe(&self) -> Result<(), BatchError> {
        let observable_json = self.accumulator.to_json().map_err(|err| {
            BatchError::layout(format!(
                "failed to serialize batch accumulator payload: {err}"
            ))
        })?;
        AccumulatorState::from_json(&observable_json).map_err(|err| {
            BatchError::layout(format!("batch accumulator is not JSON-safe: {err}"))
        })?;
        Ok(())
    }

    pub fn values_from_json(
        values: Option<&JsonValue>,
        accumulator: &JsonValue,
    ) -> Result<Self, BatchError> {
        let parsed_values = match values {
            Some(values) if !values.is_null() => {
                Some(serde_json::from_value(values.clone()).map_err(|err| {
                    BatchError::layout(format!("invalid batch values payload: {err}"))
                })?)
            }
            _ => None,
        };
        let parsed_observable = serde_json::from_value(accumulator.clone()).map_err(|err| {
            BatchError::layout(format!("invalid batch accumulator payload: {err}"))
        })?;
        Ok(Self::new(parsed_values, parsed_observable))
    }

    pub fn values_from_bytes(
        values: Option<&[u8]>,
        accumulator: &JsonValue,
    ) -> Result<Self, BatchError> {
        let parsed_values = match values {
            Some(values) => {
                let (decoded, _): (Vec<f64>, usize) =
                    bincode::serde::decode_from_slice(values, Self::binary_config()).map_err(
                        |err| BatchError::layout(format!("invalid batch values payload: {err}")),
                    )?;
                Some(decoded)
            }
            None => None,
        };
        let parsed_observable = serde_json::from_value(accumulator.clone()).map_err(|err| {
            BatchError::layout(format!("invalid batch accumulator payload: {err}"))
        })?;
        Ok(Self::new(parsed_values, parsed_observable))
    }
}

#[cfg(test)]
mod tests {
    use super::BatchResult;
    use crate::evaluation::{AccumulatorState, FullVectorAccumulatorState};

    #[test]
    fn validate_json_safe_rejects_non_finite_full_accumulator_values() {
        let result = BatchResult::new(
            None,
            AccumulatorState::FullVector(FullVectorAccumulatorState {
                components: vec!["value".to_string()],
                values_row_major: vec![1.0, f64::NAN],
                invalid_entries: vec![],
            }),
        );

        let err = result
            .validate_json_safe()
            .expect_err("expected non-finite error");
        assert!(
            err.to_string()
                .contains("batch accumulator is not JSON-safe")
        );
    }

    #[test]
    fn training_values_roundtrip_binary() {
        let result = BatchResult::new(Some(vec![1.0, 2.0, 3.5]), AccumulatorState::empty_scalar());
        let bytes = result.values_to_bytes().expect("encode values");
        let restored = BatchResult::values_from_bytes(
            bytes.as_deref(),
            &result.accumulator.to_json().expect("accumulator json"),
        )
        .expect("decode values");
        assert_eq!(restored.values, result.values);
    }

    #[test]
    fn validate_json_safe_allows_non_finite_training_values() {
        let result = BatchResult::new(
            Some(vec![1.0, f64::NAN, f64::INFINITY]),
            AccumulatorState::empty_scalar(),
        );

        result
            .validate_json_safe()
            .expect("training values are stored as binary, not json");
    }
}
