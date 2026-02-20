//! Batch abstraction for work queue operations.
//!
//! Batches are the unit of work exchanged between sampler-aggregator and evaluator.

use ndarray::Array2;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{error::Error, fmt};

/// Status of a batch in the work queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
}

impl BatchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BatchStatus::Pending => "pending",
            BatchStatus::Claimed => "claimed",
            BatchStatus::Completed => "completed",
            BatchStatus::Failed => "failed",
        }
    }
}

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BatchJson {
    continuous_rows: usize,
    continuous_cols: usize,
    continuous_data: Vec<f64>,
    discrete_rows: usize,
    discrete_cols: usize,
    discrete_data: Vec<i64>,
}

impl Batch {
    /// Constructs a batch from dense 2D arrays.
    pub fn new(continuous: Array2<f64>, discrete: Array2<i64>) -> Result<Self, BatchError> {
        if continuous.nrows() != discrete.nrows() {
            return Err(BatchError::layout(format!(
                "row count mismatch: continuous has {}, discrete has {}",
                continuous.nrows(),
                discrete.nrows()
            )));
        }
        Ok(Self {
            continuous,
            discrete,
        })
    }

    /// Constructs a batch from flat row-major payloads.
    pub fn from_flat_data(
        samples: usize,
        continuous_dims: usize,
        discrete_dims: usize,
        continuous_data: Vec<f64>,
        discrete_data: Vec<i64>,
    ) -> Result<Self, BatchError> {
        let continuous = Array2::from_shape_vec((samples, continuous_dims), continuous_data)
            .map_err(|err| {
                BatchError::layout(format!("invalid continuous payload shape: {err}"))
            })?;
        let discrete = Array2::from_shape_vec((samples, discrete_dims), discrete_data)
            .map_err(|err| BatchError::layout(format!("invalid discrete payload shape: {err}")))?;
        Self::new(continuous, discrete)
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
        Self::new(continuous, discrete)
    }
}

/// A batch with metadata from the database.
#[derive(Debug, Clone)]
pub struct BatchRecord {
    pub id: i64,
    pub run_id: i32,
    pub batch: Batch,
    pub status: BatchStatus,
    pub claimed_by: Option<String>,
}

/// Evaluator output for one batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub values: Vec<f64>,
    #[serde(default)]
    pub observable: JsonValue,
}

impl BatchResult {
    pub fn new(values: Vec<f64>, observable: JsonValue) -> Self {
        Self { values, observable }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn matches_batch(&self, batch: &Batch) -> bool {
        self.len() == batch.size()
    }

    pub fn values_to_json(&self) -> JsonValue {
        serde_json::to_value(&self.values).expect("batch values serialization should never fail")
    }

    pub fn values_from_json(
        values: &JsonValue,
        observable: &JsonValue,
    ) -> Result<Self, BatchError> {
        let parsed_values: Vec<f64> = serde_json::from_value(values.clone())
            .map_err(|err| BatchError::layout(format!("invalid batch values payload: {err}")))?;
        Ok(Self::new(parsed_values, observable.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    #[test]
    fn test_batch_creation() {
        let batch = Batch::new(array![[0.5], [1.5]], Array2::zeros((2, 0))).expect("batch");
        assert_eq!(batch.size(), 2);
        assert!(!batch.is_empty());
        assert_eq!(
            batch.point_spec(),
            PointSpec {
                continuous_dims: 1,
                discrete_dims: 0
            }
        );
    }

    #[test]
    fn test_batch_serialization() {
        let batch =
            Batch::new(array![[0.5], [1.5]], Array2::zeros((2, 0))).expect("batch creation");
        let json = batch.to_json();
        let deserialized = Batch::from_json(&json).unwrap();
        assert_eq!(deserialized.size(), batch.size());
        assert_eq!(deserialized.point_spec(), batch.point_spec());
    }

    #[test]
    fn test_batch_results() {
        let batch =
            Batch::new(array![[0.5], [1.5]], Array2::zeros((2, 0))).expect("batch creation");
        let result = BatchResult::new(vec![0.123, 0.456], JsonValue::Null);
        assert!(result.matches_batch(&batch));
    }

    #[test]
    fn test_batch_point_spec_validation() {
        let batch = Batch::new(array![[0.1, 0.2], [0.3, 0.4]], array![[1], [2]]).unwrap();
        let spec = PointSpec {
            continuous_dims: 2,
            discrete_dims: 1,
        };
        assert!(batch.validate_point_spec(&spec).is_ok());
    }

    #[test]
    fn test_batch_status() {
        assert_eq!(BatchStatus::Pending.as_str(), "pending");
        assert_eq!(BatchStatus::Claimed.as_str(), "claimed");
        assert_eq!(BatchStatus::Completed.as_str(), "completed");
        assert_eq!(BatchStatus::Failed.as_str(), "failed");
    }
}
