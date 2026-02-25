mod test_only_sin;
mod test_only_sinc;

use super::{BuildError, BuildFromJson, EvalError, ObservableEngine};
use crate::{
    batch::{Batch, BatchResult, PointSpec},
    engines::{evaluator::test_only_sinc::TestSincEvaluator, observable::ObservableFactory},
};
use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use strum::{AsRefStr, Display};

use self::test_only_sin::TestSinEvaluator;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EvaluatorImplementation {
    TestOnlySin,
    TestOnlySinc,
}

/// Evaluates integrand values for sample points.
#[enum_dispatch]
pub trait Evaluator: Send + Sync {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn eval_batch(
        &self,
        batch: &Batch,
        observable_factory: &ObservableFactory,
    ) -> Result<BatchResult, EvalError>;
    fn supports_observable(&self, observable: &ObservableEngine) -> bool;
    fn get_diagnostics(&self) -> JsonValue {
        json!("{}")
    }
}

#[enum_dispatch(Evaluator)]
pub enum EvaluatorEngine {
    TestOnlySin(TestSinEvaluator),
    TestOnlySinc(TestSincEvaluator),
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
            EvaluatorImplementation::TestOnlySinc => {
                Ok(Self::TestOnlySinc(TestSincEvaluator::from_json(params)?))
            }
        }
    }
}
