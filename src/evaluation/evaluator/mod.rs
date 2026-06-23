#[cfg(feature = "gammaloop")]
pub(crate) mod gammaloop;
#[cfg(not(feature = "gammaloop"))]
#[path = "gammaloop_disabled.rs"]
pub(crate) mod gammaloop;
pub(crate) mod process;
mod symbolica;
pub(crate) mod unit;

use crate::core::{AccumulatorConfig, BuildError, EvaluatorConfig};
use crate::evaluation::{AccumulatorState, Evaluator, SemanticAccumulatorKind};
use crate::utils::domain::Domain;

use self::gammaloop::GammaLoopEvaluator;
use self::process::ProcessEvaluator;
use self::symbolica::SymbolicaEngine;
use self::unit::UnitEvaluator;
pub use gammaloop::GammaLoopParams;
pub use process::{ProcessAccumulatorKind, ProcessEvaluatorParams};
pub use symbolica::SymbolicaParams;
pub use unit::UnitEvaluatorParams;

impl EvaluatorConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Gammaloop { .. } => "gammaloop",
            Self::Unit { .. } => "unit",
            Self::Symbolica { .. } => "symbolica",
            Self::ProcessEvaluator { .. } => "process_evaluator",
        }
    }

    pub fn build(&self) -> Result<Box<dyn Evaluator>, BuildError> {
        match self {
            Self::Gammaloop { params } => {
                Ok(Box::new(GammaLoopEvaluator::from_params(params.clone())?))
            }
            Self::Unit { params } => Ok(Box::new(UnitEvaluator::from_params(params.clone())?)),
            Self::Symbolica { params } => {
                Ok(Box::new(SymbolicaEngine::from_params(params.clone())?))
            }
            Self::ProcessEvaluator { params } => {
                Ok(Box::new(ProcessEvaluator::from_params(params.clone())?))
            }
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
            Self::ProcessEvaluator { params } => Ok(params.domain.clone()),
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
            Self::ProcessEvaluator { params } => {
                if matches!(params.accumulator, process::ProcessAccumulatorKind::Gammaloop) {
                    match config {
                        AccumulatorConfig::Empty | AccumulatorConfig::Gammaloop => Ok(()),
                        other => Err(BuildError::invalid_input(format!(
                            "evaluator.kind = \"{}\" with accumulator = \"gammaloop\" requires \
                             gammaloop accumulator config, got \"{}\"",
                            self.kind_str(),
                            other.kind_str()
                        ))),
                    }
                } else {
                    validate_process_accumulator(self.kind_str(), config, &params.components)
                }
            }
            Self::Symbolica { .. } => validate_symbolica_accumulator(self.kind_str(), config),
            Self::Unit { .. } => validate_semantic_accumulator(
                self.kind_str(),
                config,
                SemanticAccumulatorKind::Scalar,
            ),
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

fn validate_symbolica_accumulator(
    evaluator_kind: &str,
    config: &AccumulatorConfig,
) -> Result<(), BuildError> {
    if matches!(config, AccumulatorConfig::Empty) {
        return Ok(());
    }

    let supported = match config {
        AccumulatorConfig::Scalar { .. } => true,
        AccumulatorConfig::Vector { components, .. } => {
            components == &["real".to_string(), "imag".to_string()]
        }
        AccumulatorConfig::FullVector { components } => {
            components == &["value".to_string()]
                || components == &["real".to_string(), "imag".to_string()]
        }
        _ => false,
    };
    if supported {
        Ok(())
    } else {
        Err(BuildError::invalid_input(format!(
            "evaluator.kind = \"{evaluator_kind}\" supports scalar accumulators, or vector/full_vector components [\"value\"] or [\"real\", \"imag\"], got \"{}\"",
            accumulator_config_str(config)
        )))
    }
}

fn validate_process_accumulator(
    evaluator_kind: &str,
    config: &AccumulatorConfig,
    components: &[String],
) -> Result<(), BuildError> {
    match config {
        AccumulatorConfig::Empty => Ok(()),
        AccumulatorConfig::Vector {
            components: accumulator_components,
            ..
        }
        | AccumulatorConfig::FullVector {
            components: accumulator_components,
        } if accumulator_components == components => Ok(()),
        AccumulatorConfig::Vector {
            components: accumulator_components,
            ..
        }
        | AccumulatorConfig::FullVector {
            components: accumulator_components,
        } => Err(BuildError::invalid_input(format!(
            "evaluator.kind = \"{evaluator_kind}\" components {:?} do not match accumulator components {:?}",
            components, accumulator_components
        ))),
        other => Err(BuildError::invalid_input(format!(
            "evaluator.kind = \"{evaluator_kind}\" requires accumulator config \"vector\" or \"full_vector\", got \"{}\"",
            accumulator_config_str(other)
        ))),
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
        SemanticAccumulatorKind::Scalar => match config {
            AccumulatorConfig::Scalar { .. } => true,
            AccumulatorConfig::FullVector { components } => components == &["value".to_string()],
            _ => false,
        },
        SemanticAccumulatorKind::Vector => match config {
            AccumulatorConfig::Vector { .. } => true,
            AccumulatorConfig::FullVector { components } => {
                components == &["real".to_string(), "imag".to_string()]
            }
            _ => false,
        },
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

#[cfg(all(test, not(feature = "gammaloop")))]
mod no_gammaloop_tests {
    use super::{GammaLoopEvaluator, GammaLoopParams};

    #[test]
    fn gammaloop_build_reports_disabled_feature() {
        let err = match GammaLoopEvaluator::from_params(GammaLoopParams::default()) {
            Ok(_) => panic!("gammaloop should not build without the feature"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("requires a gammaboard build with the default \"gammaloop\" feature")
        );
    }
}
