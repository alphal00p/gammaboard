#[cfg(feature = "gammaloop")]
pub(crate) mod gammaloop;
#[cfg(not(feature = "gammaloop"))]
#[path = "gammaloop_disabled.rs"]
pub(crate) mod gammaloop;
mod gammaloop_config;
pub(crate) mod process;
mod symbolica;
pub(crate) mod unit;

use crate::core::{AccumulatorConfig, EngineError, EvaluatorConfig};
use crate::evaluation::Evaluator;
use crate::utils::domain::Domain;

use self::gammaloop::GammaLoopEvaluator;
use self::process::ProcessEvaluator;
use self::symbolica::SymbolicaEngine;
use self::unit::UnitEvaluator;
pub use gammaloop::GammaLoopParams;
pub use process::{ProcessEvaluatorParams, ProcessOutput};
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

    pub fn build(&self) -> Result<Box<dyn Evaluator>, EngineError> {
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

    pub fn resolve_domain(&self) -> Result<Domain, EngineError> {
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
    ) -> Result<(), EngineError> {
        match self {
            Self::ProcessEvaluator { params } => match &params.output {
                process::ProcessOutput::Gammaloop => {
                    if matches!(config, AccumulatorConfig::Gammaloop) {
                        Ok(())
                    } else {
                        Err(EngineError::invalid_input(
                            "GammaLoop process output requires a gammaloop accumulator",
                        ))
                    }
                }
                process::ProcessOutput::Values { components } => {
                    validate_value_accumulator(self.kind_str(), config, components)
                }
            },
            Self::Symbolica { .. } => validate_symbolica_accumulator(self.kind_str(), config),
            Self::Unit { .. } if matches!(config, AccumulatorConfig::Empty) => Ok(()),
            Self::Unit { .. } => {
                validate_value_accumulator(self.kind_str(), config, &["value".into()])
            }
            Self::Gammaloop { .. } => Ok(()),
        }
    }
}

fn validate_symbolica_accumulator(
    evaluator_kind: &str,
    config: &AccumulatorConfig,
) -> Result<(), EngineError> {
    if matches!(config, AccumulatorConfig::Empty) {
        return Ok(());
    }

    let supported = match config {
        AccumulatorConfig::Vector { components, .. }
        | AccumulatorConfig::FullVector { components } => {
            components == &["value".to_string()]
                || components == &["real".to_string(), "imag".to_string()]
        }
        _ => false,
    };
    if supported {
        Ok(())
    } else {
        Err(EngineError::invalid_input(format!(
            "evaluator.kind = \"{evaluator_kind}\" supports scalar accumulators, or vector/full_vector components [\"value\"] or [\"real\", \"imag\"], got \"{}\"",
            config.kind_str()
        )))
    }
}

fn validate_value_accumulator(
    evaluator_kind: &str,
    config: &AccumulatorConfig,
    components: &[String],
) -> Result<(), EngineError> {
    match config {
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
        } => Err(EngineError::invalid_input(format!(
            "evaluator.kind = \"{evaluator_kind}\" components {:?} do not match accumulator components {:?}",
            components, accumulator_components
        ))),
        other => Err(EngineError::invalid_input(format!(
            "evaluator.kind = \"{evaluator_kind}\" requires accumulator config \"vector\" or \"full_vector\", got \"{}\"",
            other.kind_str()
        ))),
    }
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

#[cfg(test)]
mod output_tests {
    use super::*;

    #[test]
    fn process_output_contract_accepts_both_storage_modes_and_rejects_mismatches() {
        let evaluator: EvaluatorConfig = serde_json::from_value(serde_json::json!({
            "kind": "process_evaluator", "command": ["worker"],
            "domain": Domain::continuous(1),
            "output": {"kind": "values", "components": ["value"]}
        }))
        .unwrap();
        for config in [
            AccumulatorConfig::scalar(),
            AccumulatorConfig::FullVector {
                components: vec!["value".into()],
            },
        ] {
            evaluator.validate_accumulator_config(&config).unwrap();
        }
        for config in [
            AccumulatorConfig::Empty,
            AccumulatorConfig::Gammaloop,
            AccumulatorConfig::FullVector {
                components: vec!["other".into()],
            },
        ] {
            assert!(evaluator.validate_accumulator_config(&config).is_err());
        }
    }
}
