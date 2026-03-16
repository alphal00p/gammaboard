pub(crate) mod gammaloop;
mod sin_evaluator;
mod sinc_evaluator;
mod symbolica;
pub(crate) mod unit;

use crate::engines::evaluation::Evaluator;
use crate::engines::{BuildError, ObservableState};

use self::gammaloop::GammaLoopEvaluator;
use self::sin_evaluator::SinEvaluator;
use self::sinc_evaluator::SincEvaluator;
use self::symbolica::SymbolicaEngine;
use self::unit::UnitEvaluator;
pub use gammaloop::GammaLoopParams;
pub use sin_evaluator::SinEvaluatorParams;
pub use sinc_evaluator::SincEvaluatorParams;
pub use symbolica::SymbolicaParams;
pub use unit::UnitEvaluatorParams;

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
            Self::Gammaloop { params } => {
                Ok(Box::new(GammaLoopEvaluator::from_params(params.clone())?))
            }
            Self::SinEvaluator { params } => {
                Ok(Box::new(SinEvaluator::from_params(params.clone())))
            }
            Self::SincEvaluator { params } => {
                Ok(Box::new(SincEvaluator::from_params(params.clone())))
            }
            Self::Unit { params } => Ok(Box::new(UnitEvaluator::from_params(params.clone()))),
            Self::Symbolica { params } => {
                Ok(Box::new(SymbolicaEngine::from_params(params.clone())?))
            }
        }
    }

    pub fn empty_observable_state(&self) -> Result<ObservableState, BuildError> {
        match self {
            Self::Gammaloop { params } => Ok(params.observable_kind.empty_state()),
            Self::SinEvaluator { .. } => Ok(ObservableState::empty_scalar()),
            Self::SincEvaluator { .. } => Ok(ObservableState::empty_complex()),
            Self::Unit { params } => Ok(params.observable_kind.empty_state()),
            Self::Symbolica { .. } => Ok(ObservableState::empty_scalar()),
        }
    }
}
