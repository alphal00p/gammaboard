use super::{Accumulator, IngestComplex, IngestScalar};
use crate::evaluation::batch::Point;
use num::complex::Complex64;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FullScalarAccumulatorState {
    pub values: Vec<f64>,
    #[serde(default)]
    pub nan_entries: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FullComplexAccumulatorState {
    pub values: Vec<ComplexValue>,
    #[serde(default)]
    pub nan_entries: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct ComplexValue {
    pub re: f64,
    pub im: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct FullAccumulatorProgress {
    pub processed: usize,
}

impl FullScalarAccumulatorState {
    pub fn push(&mut self, value: f64) {
        if !value.is_finite() {
            self.nan_entries.push(self.values.len());
            self.values.push(0.0);
            return;
        }
        self.values.push(value);
    }
}

impl IngestScalar for FullScalarAccumulatorState {
    fn ingest_scalar(&mut self, value: f64, point: &Point) {
        self.push(value * point.total_weight().abs());
    }
}

impl FullComplexAccumulatorState {
    pub fn push(&mut self, value: ComplexValue) {
        if !value.re.is_finite() || !value.im.is_finite() {
            self.nan_entries.push(self.values.len());
            self.values.push(ComplexValue::default());
            return;
        }
        self.values.push(value);
    }
}

impl IngestComplex for FullComplexAccumulatorState {
    fn ingest_complex(&mut self, value: Complex64, point: &Point) {
        let weight = point.total_weight().abs();
        self.push(ComplexValue {
            re: value.re * weight,
            im: value.im * weight,
        });
    }
}

impl Accumulator for FullScalarAccumulatorState {
    type Persistent = FullAccumulatorProgress;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        self.values.len() as i64
    }

    fn merge(&mut self, other: Self) {
        let offset = self.values.len();
        self.values.extend(other.values);
        self.nan_entries
            .extend(other.nan_entries.into_iter().map(|index| index + offset));
    }

    fn get_persistent(&self) -> Self::Persistent {
        FullAccumulatorProgress {
            processed: self.values.len(),
        }
    }
}

impl Accumulator for FullComplexAccumulatorState {
    type Persistent = FullAccumulatorProgress;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        self.values.len() as i64
    }

    fn merge(&mut self, other: Self) {
        let offset = self.values.len();
        self.values.extend(other.values);
        self.nan_entries
            .extend(other.nan_entries.into_iter().map(|index| index + offset));
    }

    fn get_persistent(&self) -> Self::Persistent {
        FullAccumulatorProgress {
            processed: self.values.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ComplexValue, FullComplexAccumulatorState, FullScalarAccumulatorState};
    use crate::evaluation::{Accumulator, IngestComplex, IngestScalar, Point};
    use num::complex::Complex64;

    #[test]
    fn full_scalar_preserves_positions_for_non_finite_values() {
        let mut accumulator = FullScalarAccumulatorState::default();

        let finite_point = Point::new(vec![], vec![], 2.0);
        let nan_point = Point::new(vec![], vec![], 1.0);
        let inf_point = Point::new(vec![], vec![], f64::INFINITY);
        accumulator.ingest_scalar(1.0, &finite_point);
        accumulator.ingest_scalar(f64::NAN, &nan_point);
        accumulator.ingest_scalar(1.0, &inf_point);

        assert_eq!(accumulator.values, vec![2.0, 0.0, 0.0]);
        assert_eq!(accumulator.nan_entries, vec![1, 2]);
    }

    #[test]
    fn full_complex_preserves_positions_for_non_finite_values() {
        let mut accumulator = FullComplexAccumulatorState::default();

        let finite_point = Point::new(vec![], vec![], 3.0);
        let nan_point = Point::new(vec![], vec![], 1.0);
        let inf_point = Point::new(vec![], vec![], f64::INFINITY);
        accumulator.ingest_complex(Complex64::new(1.0, -2.0), &finite_point);
        accumulator.ingest_complex(Complex64::new(f64::NAN, 0.0), &nan_point);
        accumulator.ingest_complex(Complex64::new(1.0, 0.0), &inf_point);

        assert_eq!(
            accumulator.values,
            vec![
                ComplexValue { re: 3.0, im: -6.0 },
                ComplexValue::default(),
                ComplexValue::default(),
            ]
        );
        assert_eq!(accumulator.nan_entries, vec![1, 2]);
    }

    #[test]
    fn full_scalar_merge_offsets_nan_entry_positions() {
        let mut left = FullScalarAccumulatorState {
            values: vec![1.0, 0.0],
            nan_entries: vec![1],
        };
        let right = FullScalarAccumulatorState {
            values: vec![2.0, 0.0, 3.0],
            nan_entries: vec![1],
        };

        left.merge(right);

        assert_eq!(left.values, vec![1.0, 0.0, 2.0, 0.0, 3.0]);
        assert_eq!(left.nan_entries, vec![1, 3]);
    }

    #[test]
    fn full_complex_merge_offsets_nan_entry_positions() {
        let mut left = FullComplexAccumulatorState {
            values: vec![ComplexValue { re: 1.0, im: 1.0 }],
            nan_entries: vec![],
        };
        let right = FullComplexAccumulatorState {
            values: vec![ComplexValue::default(), ComplexValue { re: 2.0, im: -2.0 }],
            nan_entries: vec![0],
        };

        left.merge(right);

        assert_eq!(
            left.values,
            vec![
                ComplexValue { re: 1.0, im: 1.0 },
                ComplexValue::default(),
                ComplexValue { re: 2.0, im: -2.0 },
            ]
        );
        assert_eq!(left.nan_entries, vec![1]);
    }
}
