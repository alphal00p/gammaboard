//! Batch abstraction for concrete evaluator-side materialized work.

use bincode::config::{Configuration, standard};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{error::Error, fmt};

use crate::evaluation::ObservableState;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub continuous: Vec<f64>,
    pub discrete: Vec<i64>,
    pub weight: f64,
}

impl Point {
    pub fn new(continuous: Vec<f64>, discrete: Vec<i64>, weight: f64) -> Self {
        Self {
            continuous,
            discrete,
            weight,
        }
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
        self.points.iter().map(|point| point.weight).collect()
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
    pub observable: ObservableState,
}

impl BatchResult {
    fn binary_config() -> Configuration {
        standard()
    }

    pub fn new(values: Option<Vec<f64>>, observable: ObservableState) -> Self {
        Self { values, observable }
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
        if let Some(values) = &self.values {
            for (idx, value) in values.iter().enumerate() {
                if !value.is_finite() {
                    return Err(BatchError::layout(format!(
                        "batch values contain non-finite f64 at index {idx}: {value}"
                    )));
                }
            }
        }

        let observable_json = self.observable.to_json().map_err(|err| {
            BatchError::layout(format!(
                "failed to serialize batch observable payload: {err}"
            ))
        })?;
        ObservableState::from_json(&observable_json).map_err(|err| {
            BatchError::layout(format!("batch observable is not JSON-safe: {err}"))
        })?;
        Ok(())
    }

    pub fn values_from_json(
        values: Option<&JsonValue>,
        observable: &JsonValue,
    ) -> Result<Self, BatchError> {
        let parsed_values = match values {
            Some(values) if !values.is_null() => {
                Some(serde_json::from_value(values.clone()).map_err(|err| {
                    BatchError::layout(format!("invalid batch values payload: {err}"))
                })?)
            }
            _ => None,
        };
        let parsed_observable = serde_json::from_value(observable.clone()).map_err(|err| {
            BatchError::layout(format!("invalid batch observable payload: {err}"))
        })?;
        Ok(Self::new(parsed_values, parsed_observable))
    }

    pub fn values_from_bytes(
        values: Option<&[u8]>,
        observable: &JsonValue,
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
        let parsed_observable = serde_json::from_value(observable.clone()).map_err(|err| {
            BatchError::layout(format!("invalid batch observable payload: {err}"))
        })?;
        Ok(Self::new(parsed_values, parsed_observable))
    }
}

#[cfg(test)]
mod tests {
    use super::BatchResult;
    use crate::evaluation::{FullScalarObservableState, ObservableState};

    #[test]
    fn validate_json_safe_rejects_non_finite_full_observable_values() {
        let result = BatchResult::new(
            None,
            ObservableState::FullScalar(FullScalarObservableState {
                values: vec![1.0, f64::NAN],
            }),
        );

        let err = result
            .validate_json_safe()
            .expect_err("expected non-finite error");
        assert!(
            err.to_string()
                .contains("batch observable is not JSON-safe")
        );
    }

    #[test]
    fn training_values_roundtrip_binary() {
        let result = BatchResult::new(Some(vec![1.0, 2.0, 3.5]), ObservableState::empty_scalar());
        let bytes = result.values_to_bytes().expect("encode values");
        let restored = BatchResult::values_from_bytes(
            bytes.as_deref(),
            &result.observable.to_json().expect("observable json"),
        )
        .expect("decode values");
        assert_eq!(restored.values, result.values);
    }
}
