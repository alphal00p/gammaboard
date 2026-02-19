//! Batch abstraction for work queue operations.
//!
//! This module provides types for working with batches of samples in the
//! adaptive integration system. Batches are the fundamental unit of work
//! that get distributed to workers.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{error::Error, fmt};

/// Status of a batch in the work queue
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchStatus {
    /// Batch is waiting to be claimed by a worker
    Pending,
    /// Batch has been claimed by a worker but not yet completed
    Claimed,
    /// Batch has been successfully evaluated
    Completed,
    /// Batch evaluation failed
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
    pub fn validate_point(&self, point: &Point) -> Result<(), BatchError> {
        if point.continuous.len() != self.continuous_dims {
            return Err(BatchError::layout(format!(
                "continuous dimension mismatch: expected {}, got {}",
                self.continuous_dims,
                point.continuous.len()
            )));
        }
        if point.discrete.len() != self.discrete_dims {
            return Err(BatchError::layout(format!(
                "discrete dimension mismatch: expected {}, got {}",
                self.discrete_dims,
                point.discrete.len()
            )));
        }
        Ok(())
    }

    fn expected_flat_lens(&self, samples: usize) -> Result<(usize, usize), BatchError> {
        let continuous = self
            .continuous_dims
            .checked_mul(samples)
            .ok_or_else(|| BatchError::layout("continuous flattened length overflow"))?;
        let discrete = self
            .discrete_dims
            .checked_mul(samples)
            .ok_or_else(|| BatchError::layout("discrete flattened length overflow"))?;
        Ok((continuous, discrete))
    }
}

/// Typed sample point used by sampler and evaluator engines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    #[serde(default)]
    pub continuous: Vec<f64>,
    #[serde(default)]
    pub discrete: Vec<i64>,
}

impl Point {
    pub fn new(continuous: Vec<f64>, discrete: Vec<i64>) -> Self {
        Self {
            continuous,
            discrete,
        }
    }

    pub fn scalar_continuous(value: f64) -> Self {
        Self::new(vec![value], Vec::new())
    }
}

/// Borrowed view into a single point inside a batch.
#[derive(Debug, Clone, Copy)]
pub struct PointView<'a> {
    continuous: &'a [f64],
    discrete: &'a [i64],
}

impl<'a> PointView<'a> {
    fn new(continuous: &'a [f64], discrete: &'a [i64]) -> Self {
        Self {
            continuous,
            discrete,
        }
    }

    pub fn continuous(self) -> &'a [f64] {
        self.continuous
    }

    pub fn discrete(self) -> &'a [i64] {
        self.discrete
    }

    pub fn to_owned(self) -> Point {
        Point::new(self.continuous.to_vec(), self.discrete.to_vec())
    }
}

/// A single sample point with its importance weight
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightedPoint {
    /// Typed point payload for evaluator input.
    pub point: Point,
    /// Importance weight from adaptive sampler
    pub weight: f64,
}

impl WeightedPoint {
    pub fn new(point: Point, weight: f64) -> Self {
        Self { point, weight }
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

/// A batch of samples with a single guaranteed point shape.
///
/// Points are stored in compact row-major flat vectors so database JSONB writes
/// avoid repeated per-point field names and nested objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    point_spec: PointSpec,
    #[serde(default)]
    continuous: Vec<f64>,
    #[serde(default)]
    discrete: Vec<i64>,
    weights: Vec<f64>,
}

pub struct PointIter<'a> {
    batch: &'a Batch,
    index: usize,
}

impl<'a> Iterator for PointIter<'a> {
    type Item = PointView<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let point = self.batch.get_point(self.index)?;
        self.index += 1;
        Some(point)
    }
}

impl Batch {
    /// Create a new batch from weighted points.
    ///
    /// Panics if points have mixed dimensions.
    pub fn new(points: Vec<WeightedPoint>) -> Self {
        Self::try_new(points).expect("invalid weighted points for batch")
    }

    /// Fallible constructor from weighted points.
    pub fn try_new(points: Vec<WeightedPoint>) -> Result<Self, BatchError> {
        if points.is_empty() {
            return Ok(Self {
                point_spec: PointSpec {
                    continuous_dims: 0,
                    discrete_dims: 0,
                },
                continuous: Vec::new(),
                discrete: Vec::new(),
                weights: Vec::new(),
            });
        }

        let point_spec = PointSpec {
            continuous_dims: points[0].point.continuous.len(),
            discrete_dims: points[0].point.discrete.len(),
        };
        let mut continuous = Vec::new();
        let mut discrete = Vec::new();
        let mut weights = Vec::with_capacity(points.len());
        let (expected_continuous_len, expected_discrete_len) =
            point_spec.expected_flat_lens(points.len())?;
        continuous.reserve(expected_continuous_len);
        discrete.reserve(expected_discrete_len);

        for (idx, weighted_point) in points.into_iter().enumerate() {
            if let Err(err) = point_spec.validate_point(&weighted_point.point) {
                return Err(BatchError::layout(format!("point index {idx}: {err}")));
            }
            continuous.extend_from_slice(&weighted_point.point.continuous);
            discrete.extend_from_slice(&weighted_point.point.discrete);
            weights.push(weighted_point.weight);
        }

        Self::from_flat_parts(point_spec, continuous, discrete, weights)
    }

    /// Create a batch from separate point and weight vectors
    pub fn from_parts(points: Vec<Point>, weights: Vec<f64>) -> Result<Self, BatchError> {
        if points.len() != weights.len() {
            return Err(BatchError::layout(format!(
                "Point and weight count mismatch: {} vs {}",
                points.len(),
                weights.len()
            )));
        }

        if points.is_empty() {
            return Ok(Self {
                point_spec: PointSpec {
                    continuous_dims: 0,
                    discrete_dims: 0,
                },
                continuous: Vec::new(),
                discrete: Vec::new(),
                weights: Vec::new(),
            });
        }

        let point_spec = PointSpec {
            continuous_dims: points[0].continuous.len(),
            discrete_dims: points[0].discrete.len(),
        };
        let (expected_continuous_len, expected_discrete_len) =
            point_spec.expected_flat_lens(points.len())?;
        let mut continuous = Vec::with_capacity(expected_continuous_len);
        let mut discrete = Vec::with_capacity(expected_discrete_len);

        for (idx, point) in points.into_iter().enumerate() {
            if let Err(err) = point_spec.validate_point(&point) {
                return Err(BatchError::layout(format!("point index {idx}: {err}")));
            }
            continuous.extend_from_slice(&point.continuous);
            discrete.extend_from_slice(&point.discrete);
        }

        Self::from_flat_parts(point_spec, continuous, discrete, weights)
    }

    pub fn from_flat_parts(
        point_spec: PointSpec,
        continuous: Vec<f64>,
        discrete: Vec<i64>,
        weights: Vec<f64>,
    ) -> Result<Self, BatchError> {
        let batch = Self {
            point_spec,
            continuous,
            discrete,
            weights,
        };
        batch.validate_layout()?;
        Ok(batch)
    }

    pub fn point_spec(&self) -> &PointSpec {
        &self.point_spec
    }

    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    pub fn get_weight(&self, index: usize) -> Option<f64> {
        self.weights.get(index).copied()
    }

    pub fn get_point(&self, index: usize) -> Option<PointView<'_>> {
        if index >= self.size() {
            return None;
        }
        let c_start = index.checked_mul(self.point_spec.continuous_dims)?;
        let c_end = c_start.checked_add(self.point_spec.continuous_dims)?;
        let d_start = index.checked_mul(self.point_spec.discrete_dims)?;
        let d_end = d_start.checked_add(self.point_spec.discrete_dims)?;
        Some(PointView::new(
            &self.continuous[c_start..c_end],
            &self.discrete[d_start..d_end],
        ))
    }

    pub fn iter_points(&self) -> PointIter<'_> {
        PointIter {
            batch: self,
            index: 0,
        }
    }

    pub fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BatchError> {
        if self.point_spec != *point_spec {
            return Err(BatchError::layout(format!(
                "point spec mismatch: batch({}, {}) vs run({}, {})",
                self.point_spec.continuous_dims,
                self.point_spec.discrete_dims,
                point_spec.continuous_dims,
                point_spec.discrete_dims
            )));
        }
        self.validate_layout()?;
        Ok(())
    }

    /// Get the number of samples in this batch
    pub fn size(&self) -> usize {
        self.weights.len()
    }

    /// Check if batch is empty
    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    /// Convert to JSON for database storage
    pub fn to_json(&self) -> JsonValue {
        serde_json::to_value(self).expect("Batch serialization should never fail")
    }

    /// Create from JSON stored in database
    pub fn from_json(value: &JsonValue) -> Result<Self, BatchError> {
        let batch: Self = serde_json::from_value(value.clone()).map_err(BatchError::from)?;
        batch.validate_layout()?;
        Ok(batch)
    }

    fn validate_layout(&self) -> Result<(), BatchError> {
        let (expected_continuous, expected_discrete) =
            self.point_spec.expected_flat_lens(self.size())?;
        if self.continuous.len() != expected_continuous {
            return Err(BatchError::layout(format!(
                "invalid continuous payload length: expected {}, got {}",
                expected_continuous,
                self.continuous.len()
            )));
        }
        if self.discrete.len() != expected_discrete {
            return Err(BatchError::layout(format!(
                "invalid discrete payload length: expected {}, got {}",
                expected_discrete,
                self.discrete.len()
            )));
        }
        Ok(())
    }
}

/// A batch with metadata from the database
#[derive(Debug, Clone)]
pub struct BatchRecord {
    /// Database ID
    pub id: i64,
    /// Run this batch belongs to
    pub run_id: i32,
    /// The batch data
    pub batch: Batch,
    /// Current status
    pub status: BatchStatus,
    /// Worker that claimed this batch (if any)
    pub claimed_by: Option<String>,
}

/// Results from evaluating a batch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResults {
    /// Per-sample training weights consumed by sampler training logic.
    pub training_weights: Vec<f64>,
}

/// Per-sample evaluator output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatedSample {
    /// Training weight used by sampler training.
    pub weight: f64,
    /// Per-sample observable payload to be batch-aggregated by worker runner.
    #[serde(default)]
    pub observable: JsonValue,
}

impl EvaluatedSample {
    pub fn weight_only(weight: f64) -> Self {
        Self {
            weight,
            observable: JsonValue::Null,
        }
    }
}

impl BatchResults {
    pub fn new(training_weights: Vec<f64>) -> Self {
        Self { training_weights }
    }

    pub fn from_evaluated_samples(samples: &[EvaluatedSample]) -> Self {
        Self {
            training_weights: samples.iter().map(|sample| sample.weight).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.training_weights.len()
    }

    /// Check if results match the batch size
    pub fn matches_batch(&self, batch: &Batch) -> bool {
        self.len() == batch.size()
    }

    /// Convert to JSON for database storage
    pub fn to_json(&self) -> JsonValue {
        serde_json::to_value(&self.training_weights)
            .expect("Results serialization should never fail")
    }

    /// Create from JSON stored in database
    pub fn from_json(value: &JsonValue) -> Result<Self, serde_json::Error> {
        let training_weights: Vec<f64> = serde_json::from_value(value.clone())?;
        Ok(Self::new(training_weights))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_creation() {
        let points = vec![
            WeightedPoint::new(Point::new(vec![0.5], vec![]), 1.0),
            WeightedPoint::new(Point::new(vec![1.5], vec![]), 0.8),
        ];
        let batch = Batch::new(points);
        assert_eq!(batch.size(), 2);
        assert!(!batch.is_empty());
        assert_eq!(
            batch.point_spec(),
            &PointSpec {
                continuous_dims: 1,
                discrete_dims: 0
            }
        );
    }

    #[test]
    fn test_batch_from_parts() {
        let points = vec![
            Point::new(vec![0.5, 0.3], vec![]),
            Point::new(vec![1.5, 0.7], vec![]),
        ];
        let weights = vec![1.0, 0.8];
        let batch = Batch::from_parts(points, weights).unwrap();
        assert_eq!(batch.size(), 2);
    }

    #[test]
    fn test_batch_serialization() {
        let batch = Batch::from_parts(
            vec![Point::scalar_continuous(0.5), Point::scalar_continuous(1.5)],
            vec![1.0, 0.8],
        )
        .unwrap();
        let json = batch.to_json();
        let deserialized = Batch::from_json(&json).unwrap();
        assert_eq!(deserialized.size(), batch.size());
        assert_eq!(deserialized.point_spec(), batch.point_spec());
    }

    #[test]
    fn test_batch_get_point_view() {
        let batch = Batch::from_parts(
            vec![
                Point::new(vec![0.5, 1.5], vec![1]),
                Point::new(vec![2.0, 3.0], vec![2]),
            ],
            vec![1.0, 0.8],
        )
        .unwrap();
        let point = batch.get_point(1).expect("point 1 exists");
        assert_eq!(point.continuous(), &[2.0, 3.0]);
        assert_eq!(point.discrete(), &[2]);
    }

    #[test]
    fn test_batch_results() {
        let results = BatchResults::new(vec![0.123, 0.456]);
        let batch = Batch::from_parts(
            vec![Point::scalar_continuous(0.5), Point::scalar_continuous(1.5)],
            vec![1.0, 0.8],
        )
        .unwrap();
        assert!(results.matches_batch(&batch));
    }

    #[test]
    fn test_batch_point_spec_validation() {
        let batch = Batch::from_parts(
            vec![
                Point::new(vec![0.1, 0.2], vec![1]),
                Point::new(vec![0.3, 0.4], vec![2]),
            ],
            vec![1.0, 1.0],
        )
        .unwrap();
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
