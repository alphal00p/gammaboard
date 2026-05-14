use super::{Accumulator, IngestComplex, IngestScalar};
use crate::evaluation::batch::Point;
use num::complex::Complex64;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FullVectorAccumulatorState {
    pub components: Vec<String>,
    pub values_row_major: Vec<f64>,
    #[serde(default)]
    pub invalid_entries: Vec<usize>,
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
            invalid_entries: Vec::new(),
        }
    }

    pub fn push_vector(&mut self, values: &[f64]) {
        let index = self.sample_count() as usize;
        if values.len() != self.components.len() || values.iter().any(|value| !value.is_finite()) {
            self.invalid_entries.push(index);
            self.values_row_major
                .extend(std::iter::repeat_n(0.0, self.components.len()));
            return;
        }
        self.values_row_major.extend_from_slice(values);
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
    fn ingest_scalar(&mut self, value: f64, point: &Point) {
        self.push_vector(&[value * point.total_weight().abs()]);
    }
}

impl IngestComplex for FullVectorAccumulatorState {
    fn ingest_complex(&mut self, value: Complex64, point: &Point) {
        let weight = point.total_weight().abs();
        self.push_vector(&[value.re * weight, value.im * weight]);
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

    fn merge(&mut self, other: Self) {
        let offset = self.sample_count() as usize;
        self.values_row_major.extend(other.values_row_major);
        self.invalid_entries.extend(
            other
                .invalid_entries
                .into_iter()
                .map(|index| index + offset),
        );
    }

    fn get_persistent(&self) -> Self::Persistent {
        FullAccumulatorProgress {
            processed: self.sample_count() as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FullVectorAccumulatorState;
    use crate::evaluation::{Accumulator, IngestComplex, IngestScalar, Point};
    use num::complex::Complex64;

    #[test]
    fn full_vector_preserves_scalar_positions_for_non_finite_values() {
        let mut accumulator =
            FullVectorAccumulatorState::from_components(vec!["value".to_string()]);

        let finite_point = Point::new(vec![], vec![], 2.0);
        let nan_point = Point::new(vec![], vec![], 1.0);
        let inf_point = Point::new(vec![], vec![], f64::INFINITY);
        accumulator.ingest_scalar(1.0, &finite_point);
        accumulator.ingest_scalar(f64::NAN, &nan_point);
        accumulator.ingest_scalar(1.0, &inf_point);

        assert_eq!(accumulator.values_row_major, vec![2.0, 0.0, 0.0]);
        assert_eq!(accumulator.invalid_entries, vec![1, 2]);
    }

    #[test]
    fn full_vector_preserves_multi_component_positions_for_non_finite_values() {
        let mut accumulator = FullVectorAccumulatorState::from_components(vec![
            "real".to_string(),
            "imag".to_string(),
        ]);

        let finite_point = Point::new(vec![], vec![], 3.0);
        let nan_point = Point::new(vec![], vec![], 1.0);
        let inf_point = Point::new(vec![], vec![], f64::INFINITY);
        accumulator.ingest_complex(Complex64::new(1.0, -2.0), &finite_point);
        accumulator.ingest_complex(Complex64::new(f64::NAN, 0.0), &nan_point);
        accumulator.ingest_complex(Complex64::new(1.0, 0.0), &inf_point);

        assert_eq!(
            accumulator.values_row_major,
            vec![3.0, -6.0, 0.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(accumulator.invalid_entries, vec![1, 2]);
    }

    #[test]
    fn full_vector_scalar_merge_offsets_invalid_entry_positions() {
        let mut left = FullVectorAccumulatorState::from_components(vec!["value".to_string()]);
        left.values_row_major = vec![1.0, 0.0];
        left.invalid_entries = vec![1];
        let mut right = FullVectorAccumulatorState::from_components(vec!["value".to_string()]);
        right.values_row_major = vec![2.0, 0.0, 3.0];
        right.invalid_entries = vec![1];

        left.merge(right);

        assert_eq!(left.values_row_major, vec![1.0, 0.0, 2.0, 0.0, 3.0]);
        assert_eq!(left.invalid_entries, vec![1, 3]);
    }

    #[test]
    fn full_vector_multi_component_merge_offsets_invalid_entry_positions() {
        let mut left = FullVectorAccumulatorState::from_components(vec![
            "real".to_string(),
            "imag".to_string(),
        ]);
        left.values_row_major = vec![1.0, 1.0];
        let mut right = FullVectorAccumulatorState::from_components(vec![
            "real".to_string(),
            "imag".to_string(),
        ]);
        right.values_row_major = vec![0.0, 0.0, 2.0, -2.0];
        right.invalid_entries = vec![0];

        left.merge(right);

        assert_eq!(left.values_row_major, vec![1.0, 1.0, 0.0, 0.0, 2.0, -2.0]);
        assert_eq!(left.invalid_entries, vec![1]);
    }
}
