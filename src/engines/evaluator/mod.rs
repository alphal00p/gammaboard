mod symbolica;
mod test_only_sin;
mod test_only_sinc;
mod unit;

use super::{BuildError, BuildFromJson, EvalError};
use crate::{
    batch::{Batch, BatchResult, PointSpec},
    engines::observable::ObservableFactory,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use strum::{AsRefStr, Display};

use self::symbolica::SymbolicaEngine;
use self::test_only_sin::TestSinEvaluator;
use self::test_only_sinc::TestSincEvaluator;
use self::unit::UnitEvaluator;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, AsRefStr, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EvaluatorImplementation {
    TestOnlySin,
    TestOnlySinc,
    Unit,
    Symbolica,
}

/// Evaluates integrand values for sample points.
pub trait Evaluator: Send + Sync {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn eval_batch(
        &mut self,
        batch: &Batch,
        observable_factory: &ObservableFactory,
    ) -> Result<BatchResult, EvalError>;
    fn supports_observable(&self, observable_factory: &ObservableFactory) -> bool;
    fn get_init_metadata(&self) -> JsonValue {
        json!({})
    }
    fn get_diagnostics(&self) -> JsonValue {
        json!("{}")
    }
}

#[derive(Debug, Clone)]
pub struct EvaluatorFactory {
    implementation: EvaluatorImplementation,
    params: JsonValue,
}

impl EvaluatorFactory {
    pub fn new(implementation: EvaluatorImplementation, params: JsonValue) -> Self {
        Self {
            implementation,
            params,
        }
    }

    pub fn build(&self) -> Result<Box<dyn Evaluator>, BuildError> {
        match self.implementation {
            EvaluatorImplementation::TestOnlySin => {
                Ok(Box::new(TestSinEvaluator::from_json(&self.params)?))
            }
            EvaluatorImplementation::TestOnlySinc => {
                Ok(Box::new(TestSincEvaluator::from_json(&self.params)?))
            }
            EvaluatorImplementation::Unit => Ok(Box::new(UnitEvaluator::from_json(&self.params)?)),
            EvaluatorImplementation::Symbolica => {
                Ok(Box::new(SymbolicaEngine::from_json(&self.params)?))
            }
        }
    }
}
