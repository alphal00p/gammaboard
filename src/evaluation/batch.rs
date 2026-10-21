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
}

/// Evaluation information retained with an extreme contribution, never with inputs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SampleEvaluation {
    pub integrand_value_re: f64,
    pub integrand_value_im: f64,
    pub parameterization_jacobian: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticSample {
    pub point: Point,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<SampleEvaluation>,
}

impl Point {
    pub fn new(continuous: Vec<f64>, discrete: Vec<i64>, sampler_weight: f64) -> Self {
        Self {
            continuous,
            discrete,
            weight_factors: vec![WeightFactor::new("sampler_weight", sampler_weight)],
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
    /// Own materialized points. Validate their layout with `Domain::validate_batch`
    /// at the evaluator boundary; construction itself cannot fail.
    pub fn new(points: Vec<Point>) -> Self {
        Self { points }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn points(&self) -> &[Point] {
        &self.points
    }

    pub fn into_points(self) -> Vec<Point> {
        self.points
    }

    pub fn weights(&self) -> Vec<f64> {
        self.points.iter().map(Point::total_weight).collect()
    }
}

impl FromIterator<Point> for Batch {
    fn from_iter<T: IntoIterator<Item = Point>>(points: T) -> Self {
        Self::new(points.into_iter().collect())
    }
}

/// Evaluator output for one batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    /// Weighted training projection; absent when feedback was not requested.
    pub training_values: Option<Vec<f64>>,
    pub accumulator: AccumulatorState,
}

impl BatchResult {
    fn binary_config() -> Configuration {
        standard()
    }

    pub fn new(training_values: Option<Vec<f64>>, accumulator: AccumulatorState) -> Self {
        Self {
            training_values,
            accumulator,
        }
    }

    pub(crate) fn validate_for_batch(
        &self,
        nr_samples: usize,
        require_training_values: bool,
    ) -> Result<(), crate::core::EngineError> {
        self.validate_values()?;
        if self.training_values.is_some() != require_training_values {
            return Err(crate::core::EngineError::eval(
                "feedback presence does not match the batch request",
            ));
        }
        if self
            .training_values
            .as_ref()
            .is_some_and(|values| values.len() != nr_samples)
        {
            return Err(crate::core::EngineError::eval(
                "training feedback length does not match batch sample count",
            ));
        }
        // Ordinary outputs are one row per input. Observable adapters can report
        // an independent native integration (e.g. a one-trigger MadGraph run).
        if matches!(
            self.accumulator,
            AccumulatorState::Vector(_)
                | AccumulatorState::FullVector(_)
                | AccumulatorState::Stratified(_)
        ) && self.accumulator.sample_count() != nr_samples as i64
        {
            return Err(crate::core::EngineError::eval(format!(
                "accumulator counted {} samples, expected {nr_samples}",
                self.accumulator.sample_count(),
            )));
        }
        Ok(())
    }

    pub(crate) fn training_values_to_bytes(&self) -> Result<Option<Vec<u8>>, BatchError> {
        self.training_values
            .as_ref()
            .map(|values| {
                bincode::serde::encode_to_vec(values, Self::binary_config()).map_err(|err| {
                    BatchError::layout(format!("invalid batch values payload: {err}"))
                })
            })
            .transpose()
    }

    pub(crate) fn validate_values(&self) -> Result<(), crate::core::EngineError> {
        if let Some(values) = &self.training_values
            && let Some(sample) = values.iter().position(|value| !value.is_finite())
        {
            return Err(crate::core::EngineError::eval(format!(
                "sample {sample}: non-finite training feedback"
            )));
        }
        self.accumulator.validate_values()
    }

    pub(crate) fn validate_json_safe(&self) -> Result<(), BatchError> {
        self.validate_values()
            .map_err(|err| BatchError::layout(err.to_string()))?;
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

    pub(crate) fn training_values_from_bytes(
        values: Option<&[u8]>,
        accumulator: &JsonValue,
    ) -> Result<Self, BatchError> {
        let parsed_values = match values {
            Some(values) => {
                let (decoded, consumed): (Vec<f64>, usize) =
                    bincode::serde::decode_from_slice(values, Self::binary_config()).map_err(
                        |err| BatchError::layout(format!("invalid batch values payload: {err}")),
                    )?;
                if consumed != values.len() {
                    return Err(BatchError::layout("trailing bytes after training values"));
                }
                Some(decoded)
            }
            None => None,
        };
        let parsed_observable = serde_json::from_value(accumulator.clone()).map_err(|err| {
            BatchError::layout(format!("invalid batch accumulator payload: {err}"))
        })?;
        let result = Self::new(parsed_values, parsed_observable);
        result
            .validate_values()
            .map_err(|err| BatchError::layout(err.to_string()))?;
        Ok(result)
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
            }),
        );

        let err = result
            .validate_json_safe()
            .expect_err("expected non-finite error");
        assert!(err.to_string().contains("non-finite contribution"));
    }

    #[test]
    fn training_values_roundtrip_binary() {
        let result = BatchResult::new(Some(vec![1.0, 2.0, 3.5]), AccumulatorState::empty_scalar());
        let bytes = result.training_values_to_bytes().expect("encode values");
        let restored = BatchResult::training_values_from_bytes(
            bytes.as_deref(),
            &result.accumulator.to_json().expect("accumulator json"),
        )
        .expect("decode values");
        assert_eq!(restored.training_values, result.training_values);
        let mut trailing = bytes.unwrap();
        trailing.push(0);
        assert!(
            BatchResult::training_values_from_bytes(
                Some(&trailing),
                &result.accumulator.to_json().unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn validation_rejects_non_finite_training_feedback() {
        let result = BatchResult::new(
            Some(vec![1.0, f64::NAN, f64::INFINITY]),
            AccumulatorState::empty_scalar(),
        );

        result
            .validate_json_safe()
            .expect_err("invalid feedback must not be persisted");
    }
    #[test]
    fn batch_validation_rejects_silent_sample_loss_and_unsolicited_feedback() {
        let state = AccumulatorState::FullVector(FullVectorAccumulatorState {
            components: vec!["value".into()],
            values_row_major: vec![1., 2.],
        });
        let result = BatchResult::new(None, state.clone());
        assert!(result.validate_for_batch(3, false).is_err());
        assert!(result.validate_for_batch(2, true).is_err());
        result.validate_for_batch(2, false).unwrap();
        let result = BatchResult::new(Some(vec![1., 2.]), state);
        assert!(result.validate_for_batch(2, false).is_err());
        result.validate_for_batch(2, true).unwrap();
    }
    #[test]
    fn native_observable_counts_are_independent_of_trigger_count() {
        let mut state = crate::evaluation::GammaLoopAccumulatorState::default();
        for _ in 0..3 {
            state
                .estimate
                .ingest_vector(&[1., 0.], &crate::Point::new(vec![], vec![], 1.))
                .unwrap();
        }
        BatchResult::new(None, AccumulatorState::Gammaloop(state))
            .validate_for_batch(1, false)
            .unwrap();
    }
}
