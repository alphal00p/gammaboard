use super::{Accumulator, IngestScalar};
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct EmptyAccumulatorState {}

impl IngestScalar for EmptyAccumulatorState {
    fn ingest_scalar(&mut self, _value: f64, _point: &Point) {}
}

impl Accumulator for EmptyAccumulatorState {
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
    use super::EmptyAccumulatorState;
    use crate::evaluation::Accumulator;

    #[test]
    fn empty_accumulator_merges_as_no_op() {
        let mut left = EmptyAccumulatorState::default();
        left.merge(EmptyAccumulatorState::default());
        assert_eq!(left.sample_count(), 0);
    }
}
