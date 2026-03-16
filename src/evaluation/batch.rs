//! Batch abstraction for concrete evaluator-side materialized work.

use ndarray::{Array1, Array2};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{error::Error, fmt};

use crate::engines::ObservableState;

/// Point layout contract for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointSpec {
    pub continuous_dims: usize,
    pub discrete_dims: usize,
}

impl PointSpec {
    pub fn validate_dims(
        &self,
        continuous_dims: usize,
        discrete_dims: usize,
    ) -> Result<(), BatchError> {
        if continuous_dims != self.continuous_dims {
            return Err(BatchError::layout(format!(
                "continuous dimension mismatch: expected {}, got {}",
                self.continuous_dims, continuous_dims
            )));
        }
        if discrete_dims != self.discrete_dims {
            return Err(BatchError::layout(format!(
                "discrete dimension mismatch: expected {}, got {}",
                self.discrete_dims, discrete_dims
            )));
        }
        Ok(())
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

/// Homogeneous 2D batch representation of points in R^n x N^m.
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    continuous: Array2<f64>,
    discrete: Array2<i64>,
    weights: Array1<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchJson {
    continuous_rows: usize,
    continuous_cols: usize,
    continuous_data: Vec<f64>,
    discrete_rows: usize,
    discrete_cols: usize,
    discrete_data: Vec<i64>,
    weights_data: Vec<f64>,
}

impl Batch {
    pub fn new(
        continuous: Array2<f64>,
        discrete: Array2<i64>,
        weight: Option<Array1<f64>>,
    ) -> Result<Self, BatchError> {
        if continuous.nrows() != discrete.nrows() {
            return Err(BatchError::layout(format!(
                "row count mismatch: continuous has {}, discrete has {}",
                continuous.nrows(),
                discrete.nrows()
            )));
        }

        let weights = weight.unwrap_or_else(|| Array1::ones(continuous.nrows()));

        if continuous.nrows() != weights.len() {
            return Err(BatchError::layout(format!(
                "row count mismatch: continuous has {}, weights has {}",
                continuous.nrows(),
                weights.len()
            )));
        }

        Ok(Self {
            continuous,
            discrete,
            weights,
        })
    }

    pub fn new_continuous(continuous: Array2<f64>) -> Result<Self, BatchError> {
        let discrete = Array2::zeros((continuous.nrows(), 0));
        Self::new(continuous, discrete, None)
    }

    pub fn from_flat_data(
        samples: usize,
        continuous_dims: usize,
        discrete_dims: usize,
        continuous_data: Vec<f64>,
        discrete_data: Vec<i64>,
    ) -> Result<Self, BatchError> {
        Self::from_flat_data_with_weights(
            samples,
            continuous_dims,
            discrete_dims,
            continuous_data,
            discrete_data,
            None,
        )
    }

    pub fn from_flat_data_with_weights(
        samples: usize,
        continuous_dims: usize,
        discrete_dims: usize,
        continuous_data: Vec<f64>,
        discrete_data: Vec<i64>,
        weights_data: Option<Vec<f64>>,
    ) -> Result<Self, BatchError> {
        let continuous = Array2::from_shape_vec((samples, continuous_dims), continuous_data)
            .map_err(|err| {
                BatchError::layout(format!("invalid continuous payload shape: {err}"))
            })?;
        let discrete = Array2::from_shape_vec((samples, discrete_dims), discrete_data)
            .map_err(|err| BatchError::layout(format!("invalid discrete payload shape: {err}")))?;
        let weights = weights_data.map(Array1::from_vec);
        Self::new(continuous, discrete, weights)
    }

    pub fn size(&self) -> usize {
        self.continuous.nrows()
    }

    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    pub fn point_spec(&self) -> PointSpec {
        PointSpec {
            continuous_dims: self.continuous.ncols(),
            discrete_dims: self.discrete.ncols(),
        }
    }

    pub fn continuous(&self) -> &Array2<f64> {
        &self.continuous
    }

    pub fn discrete(&self) -> &Array2<i64> {
        &self.discrete
    }

    pub fn weights(&self) -> &Array1<f64> {
        &self.weights
    }

    pub fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BatchError> {
        point_spec.validate_dims(self.continuous.ncols(), self.discrete.ncols())
    }

    pub fn to_json(&self) -> JsonValue {
        let payload = BatchJson {
            continuous_rows: self.continuous.nrows(),
            continuous_cols: self.continuous.ncols(),
            continuous_data: self.continuous.iter().copied().collect(),
            discrete_rows: self.discrete.nrows(),
            discrete_cols: self.discrete.ncols(),
            discrete_data: self.discrete.iter().copied().collect(),
            weights_data: self.weights.iter().copied().collect(),
        };
        serde_json::to_value(payload).expect("Batch serialization should never fail")
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, BatchError> {
        let payload: BatchJson = serde_json::from_value(value.clone())?;
        let continuous = Array2::from_shape_vec(
            (payload.continuous_rows, payload.continuous_cols),
            payload.continuous_data,
        )
        .map_err(|err| BatchError::layout(format!("invalid continuous payload shape: {err}")))?;
        let discrete = Array2::from_shape_vec(
            (payload.discrete_rows, payload.discrete_cols),
            payload.discrete_data,
        )
        .map_err(|err| BatchError::layout(format!("invalid discrete payload shape: {err}")))?;
        let weights = Array1::from_vec(payload.weights_data);
        Self::new(continuous, discrete, Some(weights))
    }
}

/// Evaluator output for one batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub values: Option<Vec<f64>>,
    pub observable: ObservableState,
}

impl BatchResult {
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
        let parsed_observable = ObservableState::from_json(observable).map_err(|err| {
            BatchError::layout(format!("invalid batch observable payload: {err}"))
        })?;
        Ok(Self::new(parsed_values, parsed_observable))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn test_batch_creation() {
        let batch = Batch::new(array![[0.5], [1.5]], Array2::zeros((2, 0)), None).expect("batch");
        assert_eq!(batch.size(), 2);
        assert!(!batch.is_empty());
        assert_eq!(
            batch.point_spec(),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0
            }
        );
        assert_eq!(batch.weights().to_vec(), vec![1.0, 1.0]);
    }

    #[test]
    fn test_batch_serialization() {
        let batch =
            Batch::new(array![[0.5], [1.5]], Array2::zeros((2, 0)), None).expect("batch creation");
        let json = batch.to_json();
        let deserialized = Batch::from_json(&json).unwrap();
        assert_eq!(deserialized.size(), batch.size());
        assert_eq!(deserialized.point_spec(), batch.point_spec());
        assert_eq!(deserialized.weights(), batch.weights());
    }

    #[test]
    fn test_batch_results() {
        let batch =
            Batch::new(array![[0.5], [1.5]], Array2::zeros((2, 0)), None).expect("batch creation");
        let result = BatchResult::new(Some(vec![0.123, 0.456]), ObservableState::empty_scalar());
        assert!(result.matches_batch(&batch));
    }

    #[test]
    fn test_batch_point_spec_validation() {
        let batch = Batch::new(array![[0.1, 0.2], [0.3, 0.4]], array![[1], [2]], None).unwrap();
        let spec = PointSpec {
            continuous_dims: 2,
            discrete_dims: 1,
        };
        assert!(batch.validate_point_spec(&spec).is_ok());
    }
}
