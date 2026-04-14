use super::{IngestComplex, IngestScalar, Observable};
use num::complex::Complex64;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct EmptyObservableState {}

impl IngestScalar for EmptyObservableState {
    fn ingest_scalar(&mut self, _value: f64, _weight: f64) {}
}

impl IngestComplex for EmptyObservableState {
    fn ingest_complex(&mut self, _value: Complex64, _weight: f64) {}
}

impl Observable for EmptyObservableState {
    type Persistent = Self;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        0
    }

    fn merge(&mut self, _other: Self) {}

    fn get_persistent(&self) -> Self::Persistent {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::EmptyObservableState;
    use crate::evaluation::Observable;

    #[test]
    fn empty_observable_merges_as_no_op() {
        let mut left = EmptyObservableState::default();
        left.merge(EmptyObservableState::default());
        assert_eq!(left.sample_count(), 0);
    }
}
