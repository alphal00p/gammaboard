//! Batch abstraction for work queue operations.
//!
//! This module provides types for working with batches of samples in the
//! adaptive integration system. Batches are the fundamental unit of work
//! that get distributed to workers.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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

/// A single sample point with its importance weight
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeightedPoint {
    /// The point to evaluate (can be scalar, array, or object)
    pub point: JsonValue,
    /// Importance weight from adaptive sampler
    pub weight: f64,
}

impl WeightedPoint {
    pub fn new(point: JsonValue, weight: f64) -> Self {
        Self { point, weight }
    }
}

/// A batch of samples to be evaluated
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    /// Points with their importance weights
    pub points: Vec<WeightedPoint>,
}

impl Batch {
    /// Create a new batch from points and weights
    pub fn new(points: Vec<WeightedPoint>) -> Self {
        Self { points }
    }

    /// Create a batch from separate point and weight vectors
    pub fn from_parts(points: Vec<JsonValue>, weights: Vec<f64>) -> Result<Self, String> {
        if points.len() != weights.len() {
            return Err(format!(
                "Point and weight count mismatch: {} vs {}",
                points.len(),
                weights.len()
            ));
        }

        let weighted_points = points
            .into_iter()
            .zip(weights)
            .map(|(point, weight)| WeightedPoint::new(point, weight))
            .collect();

        Ok(Self::new(weighted_points))
    }

    /// Get the number of samples in this batch
    pub fn size(&self) -> usize {
        self.points.len()
    }

    /// Check if batch is empty
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Convert to JSON for database storage
    pub fn to_json(&self) -> JsonValue {
        serde_json::to_value(self).expect("Batch serialization should never fail")
    }

    /// Create from JSON stored in database
    pub fn from_json(value: &JsonValue) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value.clone())
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
    use serde_json::json;

    #[test]
    fn test_batch_creation() {
        let points = vec![
            WeightedPoint::new(json!({"x": 0.5}), 1.0),
            WeightedPoint::new(json!({"x": 1.5}), 0.8),
        ];
        let batch = Batch::new(points);
        assert_eq!(batch.size(), 2);
        assert!(!batch.is_empty());
    }

    #[test]
    fn test_batch_from_parts() {
        let points = vec![json!([0.5, 0.3]), json!([1.5, 0.7])];
        let weights = vec![1.0, 0.8];
        let batch = Batch::from_parts(points, weights).unwrap();
        assert_eq!(batch.size(), 2);
    }

    #[test]
    fn test_batch_serialization() {
        let batch = Batch::from_parts(vec![json!(0.5), json!(1.5)], vec![1.0, 0.8]).unwrap();
        let json = batch.to_json();
        let deserialized = Batch::from_json(&json).unwrap();
        assert_eq!(deserialized.size(), batch.size());
    }

    #[test]
    fn test_batch_results() {
        let results = BatchResults::new(vec![0.123, 0.456]);
        let batch = Batch::from_parts(vec![json!(0.5), json!(1.5)], vec![1.0, 0.8]).unwrap();
        assert!(results.matches_batch(&batch));
    }

    #[test]
    fn test_batch_status() {
        assert_eq!(BatchStatus::Pending.as_str(), "pending");
        assert_eq!(BatchStatus::Claimed.as_str(), "claimed");
        assert_eq!(BatchStatus::Completed.as_str(), "completed");
        assert_eq!(BatchStatus::Failed.as_str(), "failed");
    }
}
