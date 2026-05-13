pub(crate) mod gammaloop;
pub(crate) mod process;
mod symbolica;
pub(crate) mod unit;

use crate::core::{AccumulatorConfig, BuildError, EvaluatorConfig};
use crate::evaluation::{AccumulatorState, Evaluator, SemanticAccumulatorKind};
use crate::utils::domain::Domain;

use self::gammaloop::GammaLoopEvaluator;
use self::process::ProcessScalarEvaluator;
use self::symbolica::SymbolicaEngine;
use self::unit::UnitEvaluator;
pub use gammaloop::GammaLoopParams;
pub use process::ProcessScalarParams;
pub use symbolica::SymbolicaParams;
pub use unit::UnitEvaluatorParams;

impl EvaluatorConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Gammaloop { .. } => "gammaloop",
            Self::Unit { .. } => "unit",
            Self::Symbolica { .. } => "symbolica",
            Self::ProcessScalar { .. } => "process_scalar",
        }
    }

    pub fn build(&self) -> Result<Box<dyn Evaluator>, BuildError> {
        match self {
            Self::Gammaloop { params } => {
                Ok(Box::new(GammaLoopEvaluator::from_params(params.clone())?))
            }
            Self::Unit { params } => Ok(Box::new(UnitEvaluator::from_params(params.clone()))),
            Self::Symbolica { params } => {
                Ok(Box::new(SymbolicaEngine::from_params(params.clone())?))
            }
            Self::ProcessScalar { params } => Ok(Box::new(ProcessScalarEvaluator::from_params(
                params.clone(),
            )?)),
        }
    }

    pub fn resolve_domain(&self) -> Result<Domain, BuildError> {
        match self {
            Self::Gammaloop { params } => {
                GammaLoopEvaluator::resolve_domain_from_params(params.clone())
            }
            Self::Unit { params } => Ok(Domain::rectangular(
                params.continuous_dims,
                params.discrete_dims,
            )),
            Self::Symbolica { params } => Ok(Domain::continuous(params.args.len())),
            Self::ProcessScalar { params } => Ok(Domain::rectangular_with_cardinalities(
                params.continuous_dims,
                params.discrete_cardinalities.clone(),
            )),
        }
    }

    pub fn validate_accumulator_config(
        &self,
        config: &AccumulatorConfig,
    ) -> Result<(), BuildError> {
        if matches!(self, Self::Gammaloop { .. }) {
            return Ok(());
        }

        match self {
            Self::ProcessScalar { .. } | Self::Symbolica { .. } => validate_semantic_accumulator(
                self.kind_str(),
                config,
                SemanticAccumulatorKind::Scalar,
            ),
            Self::Unit { params } => {
                validate_semantic_accumulator(self.kind_str(), config, params.accumulator_kind)
            }
            Self::Gammaloop { .. } => Ok(()),
        }
    }

    pub fn empty_accumulator_state(
        &self,
        config: &AccumulatorConfig,
    ) -> Result<AccumulatorState, BuildError> {
        Ok(AccumulatorState::from_config(config))
    }
}

fn validate_semantic_accumulator(
    evaluator_kind: &str,
    config: &AccumulatorConfig,
    semantic_kind: SemanticAccumulatorKind,
) -> Result<(), BuildError> {
    if matches!(config, AccumulatorConfig::Empty) {
        return Ok(());
    }

    let supported = match semantic_kind {
        SemanticAccumulatorKind::Scalar => matches!(
            config,
            AccumulatorConfig::Scalar { .. } | AccumulatorConfig::FullScalar
        ),
        SemanticAccumulatorKind::Complex => matches!(
            config,
            AccumulatorConfig::Complex { .. } | AccumulatorConfig::FullComplex
        ),
    };
    if supported {
        Ok(())
    } else {
        Err(BuildError::invalid_input(format!(
            "evaluator.kind = \"{evaluator_kind}\" does not support accumulator config \"{}\"",
            accumulator_config_str(config)
        )))
    }
}

fn accumulator_config_str(config: &AccumulatorConfig) -> &'static str {
    config.kind_str()
}
