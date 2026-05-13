use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::core::{AccumulatorConfig, BuildError, EvalError};
use crate::evaluation::{Batch, BatchResult, EvalBatchOptions, Evaluator};
use crate::utils::domain::Domain;

pub struct GammaLoopEvaluator;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrainingProjection {
    #[default]
    Real,
    Imag,
    Abs,
    AbsSq,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GammaLoopParams {
    pub state_folder: PathBuf,
    pub process_id: Option<JsonValue>,
    pub integrand_name: Option<String>,
    pub momentum_space: bool,
    pub use_f128: bool,
    pub training_projection: TrainingProjection,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub post_load_commands: Vec<String>,
}

impl Default for GammaLoopParams {
    fn default() -> Self {
        Self {
            state_folder: PathBuf::from("./gammaloop_state"),
            process_id: None,
            integrand_name: None,
            post_load_commands: Vec::new(),
            momentum_space: false,
            use_f128: false,
            training_projection: TrainingProjection::default(),
        }
    }
}

impl GammaLoopEvaluator {
    pub fn from_params(_params: GammaLoopParams) -> Result<Self, BuildError> {
        Err(gammaloop_disabled_error())
    }

    pub fn resolve_domain_from_params(_params: GammaLoopParams) -> Result<Domain, BuildError> {
        Err(gammaloop_disabled_error())
    }
}

impl Evaluator for GammaLoopEvaluator {
    fn get_domain(&self) -> Domain {
        Domain::continuous(0)
    }

    fn eval_batch(
        &mut self,
        _batch: &Batch,
        _accumulator: &AccumulatorConfig,
        _options: EvalBatchOptions,
    ) -> Result<BatchResult, EvalError> {
        Err(EvalError::eval(
            "evaluator.kind = \"gammaloop\" requires a gammaboard build with the default \"gammaloop\" feature enabled",
        ))
    }
}

fn gammaloop_disabled_error() -> BuildError {
    BuildError::invalid_input(
        "evaluator.kind = \"gammaloop\" requires a gammaboard build with the default \"gammaloop\" feature enabled",
    )
}
