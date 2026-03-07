pub(crate) mod gammaloop;
mod sin_evaluator;
mod sinc_evaluator;
mod symbolica;
pub(crate) mod unit;

use super::{BuildError, BuildFromJson, EvalError};
use crate::{
    core::{Batch, BatchResult, PointSpec},
    engines::ObservableState,
};
use serde_json::{Value as JsonValue, json};

use self::gammaloop::GammaLoopEvaluator;
use self::sin_evaluator::SinEvaluator;
use self::sinc_evaluator::SincEvaluator;
use self::symbolica::SymbolicaEngine;
use self::unit::UnitEvaluator;

#[derive(Debug, Clone, Copy)]
pub struct EvalBatchOptions {
    pub require_training_values: bool,
}

/// Evaluates integrand values for sample points.
pub trait Evaluator: Send {
    fn get_point_spec(&self) -> PointSpec;
    fn empty_observable(&self) -> ObservableState;
    fn eval_batch(
        &mut self,
        batch: &Batch,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError>;
    fn get_init_metadata(&self) -> JsonValue {
        json!({})
    }
    fn get_diagnostics(&self) -> JsonValue {
        json!("{}")
    }
}

impl crate::engines::EvaluatorConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Gammaloop { .. } => "gammaloop",
            Self::SinEvaluator { .. } => "sin_evaluator",
            Self::SincEvaluator { .. } => "sinc_evaluator",
            Self::Unit { .. } => "unit",
            Self::Symbolica { .. } => "symbolica",
        }
    }

    pub fn build(&self) -> Result<Box<dyn Evaluator>, BuildError> {
        match self {
            Self::Gammaloop { params } => Ok(Box::new(GammaLoopEvaluator::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
            Self::SinEvaluator { params } => Ok(Box::new(SinEvaluator::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
            Self::SincEvaluator { params } => Ok(Box::new(SincEvaluator::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
            Self::Unit { params } => Ok(Box::new(UnitEvaluator::from_json(&JsonValue::Object(
                params.clone(),
            ))?)),
            Self::Symbolica { params } => Ok(Box::new(SymbolicaEngine::from_json(
                &JsonValue::Object(params.clone()),
            )?)),
        }
    }

    pub fn empty_observable_state(&self) -> Result<ObservableState, BuildError> {
        match self {
            Self::Gammaloop { params } => {
                let params: gammaloop::GammaLoopParams =
                    serde_json::from_value(JsonValue::Object(params.clone()))
                        .map_err(|err| BuildError::invalid_input(err.to_string()))?;
                Ok(params.observable_kind.empty_state())
            }
            Self::SinEvaluator { .. } => Ok(ObservableState::empty_scalar()),
            Self::SincEvaluator { .. } => Ok(ObservableState::empty_complex()),
            Self::Unit { params } => {
                let params: unit::UnitEvaluatorParams =
                    serde_json::from_value(JsonValue::Object(params.clone()))
                        .map_err(|err| BuildError::invalid_input(err.to_string()))?;
                Ok(params.observable_kind.empty_state())
            }
            Self::Symbolica { .. } => Ok(ObservableState::empty_scalar()),
        }
    }
}
