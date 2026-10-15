use super::{Accumulator, IngestScalar};
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct EmptyAccumulatorState {}

impl IngestScalar for EmptyAccumulatorState {
    fn ingest_scalar(&mut self, value: f64, point: &Point) -> Result<(), crate::core::EngineError> {
        if !(value * point.total_weight()).is_finite() {
            return Err(crate::core::EngineError::eval(
                "non-finite weighted contribution",
            ));
        }
        Ok(())
    }
}

impl Accumulator for EmptyAccumulatorState {
    type Persistent = Self;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        0
    }

    fn merge(&mut self, _other: Self) -> Result<(), crate::core::EngineError> {
        Ok(())
    }

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
        left.merge(EmptyAccumulatorState::default()).unwrap();
        assert_eq!(left.sample_count(), 0);
    }
}
