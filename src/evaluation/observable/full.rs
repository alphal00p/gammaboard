use super::{IngestComplex, IngestScalar, Observable};
use num::complex::Complex64;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FullScalarObservableState {
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct FullComplexObservableState {
    pub values: Vec<ComplexValue>,
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
        self.values.extend(other.values);
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
        self.values.extend(other.values);
    }

    fn get_persistent(&self) -> Self::Persistent {
        FullObservableProgress {
            processed: self.values.len(),
        }
    }
}
