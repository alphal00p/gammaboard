mod test_only_sin;

use super::{BuildError, BuildFromJson, EvalError, Observable, ObservableEngine};
use crate::batch::{Batch, BatchResult, PointSpec};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::fmt;

use self::test_only_sin::TestSinEvaluator;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluatorImplementation {
    TestOnlySin,
}

impl EvaluatorImplementation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TestOnlySin => "test_only_sin",
        }
    }
}

impl fmt::Display for EvaluatorImplementation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Evaluates integrand values for sample points.
#[enum_dispatch]
pub trait Evaluator: Send + Sync {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn eval_batch(
        &self,
        batch: &Batch,
        observable: &mut dyn Observable,
    ) -> Result<BatchResult, EvalError>;
    fn supports_observable(&self, observable: &ObservableEngine) -> bool;
}

#[enum_dispatch(Evaluator)]
pub enum EvaluatorEngine {
    TestOnlySin(TestSinEvaluator),
}

impl EvaluatorEngine {
    pub fn build(
        implementation: EvaluatorImplementation,
        params: &JsonValue,
    ) -> Result<Self, BuildError> {
        match implementation {
            EvaluatorImplementation::TestOnlySin => {
                Ok(Self::TestOnlySin(TestSinEvaluator::from_json(params)?))
            }
        }
    }
}
