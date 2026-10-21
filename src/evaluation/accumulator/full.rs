use super::{Accumulator, IngestScalar};
use crate::core::EngineError;
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FullVectorAccumulatorState {
    pub components: Vec<String>,
    pub values_row_major: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FullAccumulatorProgress {
    pub processed: usize,
}

impl FullVectorAccumulatorState {
    pub fn from_components(components: Vec<String>) -> Self {
        Self {
            components,
            values_row_major: Vec::new(),
        }
    }

    pub fn push_vector(&mut self, values: &[f64]) -> Result<(), EngineError> {
        if values.len() != self.components.len() {
            return Err(EngineError::eval("full-vector component count mismatch"));
        }
        if let Some(index) = values.iter().position(|value| !value.is_finite()) {
            return Err(EngineError::eval(format!(
                "non-finite component {:?}",
                self.components[index]
            )));
        }
        self.values_row_major.extend_from_slice(values);
        Ok(())
    }

    pub fn component_index(&self, name: &str) -> Option<usize> {
        self.components
            .iter()
            .position(|component| component == name)
    }

    pub fn component_values(&self, name: &str) -> Option<Vec<f64>> {
        let index = self.component_index(name)?;
        Some(
            self.values_row_major
                .chunks(self.components.len())
                .filter_map(|row| row.get(index).copied())
                .collect(),
        )
    }
}

impl IngestScalar for FullVectorAccumulatorState {
    fn ingest_scalar(&mut self, value: f64, point: &Point) -> Result<(), EngineError> {
        self.push_vector(&[value * point.total_weight().abs()])
    }
}

impl Accumulator for FullVectorAccumulatorState {
    type Persistent = FullAccumulatorProgress;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        if self.components.is_empty() {
            0
        } else {
            (self.values_row_major.len() / self.components.len()) as i64
        }
    }

    fn merge(&mut self, other: Self) -> Result<(), crate::core::EngineError> {
        let width = self.components.len();
        if self.components != other.components
            || width == 0
            || !self.values_row_major.len().is_multiple_of(width)
            || !other.values_row_major.len().is_multiple_of(width)
            || self
                .values_row_major
                .iter()
                .chain(&other.values_row_major)
                .any(|value| !value.is_finite())
        {
            return Err(crate::core::EngineError::engine(
                "incompatible or malformed full-vector accumulator",
            ));
        }
        self.values_row_major.extend(other.values_row_major);
        Ok(())
    }

    fn get_persistent(&self) -> Self::Persistent {
        FullAccumulatorProgress {
            processed: self.sample_count() as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_rows_are_rejected_without_placeholders_or_partial_rows() {
        let mut state =
            FullVectorAccumulatorState::from_components(vec!["real".into(), "imag".into()]);
        state.push_vector(&[1.0, 2.0]).unwrap();
        for values in [&[f64::NAN, 0.0][..], &[1.0, f64::INFINITY], &[1.0]] {
            assert!(state.push_vector(values).is_err());
            assert_eq!(state.values_row_major, vec![1.0, 2.0]);
        }
    }
}
