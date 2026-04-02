use super::{IngestComplex, IngestScalar, Observable};
use num::complex::Complex64;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FullScalarObservableState {
    pub values: Vec<f64>,
    #[serde(default)]
    pub nan_entries: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FullComplexObservableState {
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
pub struct FullObservableProgress {
    pub processed: usize,
}

impl FullScalarObservableState {
    pub fn push(&mut self, value: f64) {
        if !value.is_finite() {
            self.nan_entries.push(self.values.len());
            self.values.push(0.0);
            return;
        }
        self.values.push(value);
    }
}

impl IngestScalar for FullScalarObservableState {
    fn ingest_scalar(&mut self, value: f64, weight: f64) {
        self.push(value * weight.abs());
    }
}

impl FullComplexObservableState {
    pub fn push(&mut self, value: ComplexValue) {
        if !value.re.is_finite() || !value.im.is_finite() {
            self.nan_entries.push(self.values.len());
            self.values.push(ComplexValue::default());
            return;
        }
        self.values.push(value);
    }
}

impl IngestComplex for FullComplexObservableState {
    fn ingest_complex(&mut self, value: Complex64, weight: f64) {
        let weight = weight.abs();
        self.push(ComplexValue {
            re: value.re * weight,
            im: value.im * weight,
        });
    }
}

impl Observable for FullScalarObservableState {
    type Persistent = FullObservableProgress;
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
        FullObservableProgress {
            processed: self.values.len(),
        }
    }
}

impl Observable for FullComplexObservableState {
    type Persistent = FullObservableProgress;
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
        FullObservableProgress {
            processed: self.values.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ComplexValue, FullComplexObservableState, FullScalarObservableState};
    use crate::evaluation::{IngestComplex, IngestScalar, Observable};
    use num::complex::Complex64;

    #[test]
    fn full_scalar_preserves_positions_for_non_finite_values() {
        let mut observable = FullScalarObservableState::default();

        observable.ingest_scalar(1.0, 2.0);
        observable.ingest_scalar(f64::NAN, 1.0);
        observable.ingest_scalar(1.0, f64::INFINITY);

        assert_eq!(observable.values, vec![2.0, 0.0, 0.0]);
        assert_eq!(observable.nan_entries, vec![1, 2]);
    }

    #[test]
    fn full_complex_preserves_positions_for_non_finite_values() {
        let mut observable = FullComplexObservableState::default();

        observable.ingest_complex(Complex64::new(1.0, -2.0), 3.0);
        observable.ingest_complex(Complex64::new(f64::NAN, 0.0), 1.0);
        observable.ingest_complex(Complex64::new(1.0, 0.0), f64::INFINITY);

        assert_eq!(
            observable.values,
            vec![
                ComplexValue { re: 3.0, im: -6.0 },
                ComplexValue::default(),
                ComplexValue::default(),
            ]
        );
        assert_eq!(observable.nan_entries, vec![1, 2]);
    }

    #[test]
    fn full_scalar_merge_offsets_nan_entry_positions() {
        let mut left = FullScalarObservableState {
            values: vec![1.0, 0.0],
            nan_entries: vec![1],
        };
        let right = FullScalarObservableState {
            values: vec![2.0, 0.0, 3.0],
            nan_entries: vec![1],
        };

        left.merge(right);

        assert_eq!(left.values, vec![1.0, 0.0, 2.0, 0.0, 3.0]);
        assert_eq!(left.nan_entries, vec![1, 3]);
    }

    #[test]
    fn full_complex_merge_offsets_nan_entry_positions() {
        let mut left = FullComplexObservableState {
            values: vec![ComplexValue { re: 1.0, im: 1.0 }],
            nan_entries: vec![],
        };
        let right = FullComplexObservableState {
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
