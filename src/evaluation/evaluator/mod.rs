#[cfg(feature = "gammaloop")]
pub(crate) mod gammaloop;
#[cfg(not(feature = "gammaloop"))]
#[path = "gammaloop_disabled.rs"]
pub(crate) mod gammaloop;
mod gammaloop_config;
pub(crate) mod process;
#[cfg(feature = "symbolica")]
mod symbolica;
mod symbolica_config;
pub(crate) mod unit;

use crate::core::{AccumulatorConfig, EngineError, EvaluatorConfig};
use crate::evaluation::Evaluator;
use crate::utils::domain::Domain;

use self::gammaloop::GammaLoopEvaluator;
use self::process::ProcessEvaluator;
#[cfg(feature = "symbolica")]
use self::symbolica::SymbolicaEngine;
use self::unit::UnitEvaluator;
pub use gammaloop::GammaLoopParams;
pub use process::{ProcessEvaluatorParams, ProcessOutput};
pub use symbolica_config::SymbolicaParams;
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
            #[cfg(feature = "symbolica")]
            Self::Symbolica { params } => {
                Ok(Box::new(SymbolicaEngine::from_params(params.clone())?))
            }
            #[cfg(not(feature = "symbolica"))]
            Self::Symbolica { .. } => Err(EngineError::build(
                "evaluator.kind = \"symbolica\" requires the \"symbolica\" build feature",
            )),
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
            Self::ProcessEvaluator { params } => params
                .domain
                .clone()
                .map(Ok)
                .unwrap_or_else(|| params.resolve_descriptor().map(|d| d.domain)),
        }
    }

    pub fn resolve_descriptor(&self) -> Result<crate::EvaluatorDescriptor, EngineError> {
        if let Self::ProcessEvaluator { params } = self {
            return params.resolve_descriptor();
        }
        self.descriptor_for_domain(self.resolve_domain()?)
    }

    /// Reuse an already validated run domain when checking output capabilities.
    /// This must not reload a native physics state for every task preflight.
    pub fn descriptor_for_domain(
        &self,
        domain: Domain,
    ) -> Result<crate::EvaluatorDescriptor, EngineError> {
        let d = match self {
            Self::Gammaloop { params } => {
                let mut d = crate::EvaluatorDescriptor::complex(domain, params.training_projection);
                d.observables = true;
                d.details = true;
                d.diagnostics = true;
                d
            }
            Self::Unit { .. } => crate::EvaluatorDescriptor::values(domain, vec!["value".into()]),
            Self::Symbolica { params } => crate::EvaluatorDescriptor::values(
                domain,
                match &params.source {
                    symbolica_config::SymbolicaSource::Expressions { components, .. } => {
                        components.clone()
                    }
                    _ => vec!["value".into(), "real".into(), "imag".into()],
                },
            ),
            Self::ProcessEvaluator { params } => return params.resolve_descriptor(),
        };
        d.validate()?;
        Ok(d)
    }
    pub fn discover(&mut self) -> Result<crate::EvaluatorDescriptor, EngineError> {
        let descriptor = self.resolve_descriptor()?;
        if let Self::ProcessEvaluator { params } = self {
            // Explicit legacy output cards remain offline-validatable. Modern
            // cards cache discovery so each task does not spawn another probe.
            if params.output.is_none() {
                params.domain = Some(descriptor.domain.clone());
                params.descriptor = Some(descriptor.clone());
            }
        }
        Ok(descriptor)
    }
    pub fn validate_accumulator_config(
        &self,
        config: &AccumulatorConfig,
        domain: &Domain,
    ) -> Result<(), EngineError> {
        self.descriptor_for_domain(domain.clone())?.plan(config)?;
        #[cfg(not(feature = "gammaloop"))]
        if matches!(config, AccumulatorConfig::Gammaloop) {
            return Err(EngineError::build(
                "observable accumulation requires gammaloop feature",
            ));
        }
        Ok(())
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
    fn gammaloop_output_validation_reuses_the_resolved_domain() {
        let evaluator = EvaluatorConfig::Gammaloop {
            params: GammaLoopParams {
                state_folder: "/missing-state-output-validation-must-not-load".into(),
                ..Default::default()
            },
        };
        evaluator
            .validate_accumulator_config(
                &AccumulatorConfig::FullVector {
                    components: vec!["real".into()],
                },
                &Domain::continuous(3),
            )
            .unwrap();
    }

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
            evaluator
                .validate_accumulator_config(&config, &Domain::continuous(1))
                .unwrap();
        }
        for config in [
            AccumulatorConfig::Gammaloop,
            AccumulatorConfig::FullVector {
                components: vec!["other".into()],
            },
        ] {
            assert!(
                evaluator
                    .validate_accumulator_config(&config, &Domain::continuous(1))
                    .is_err()
            );
        }
    }
}
