pub use super::gammaloop_config::{GammaLoopParams, TrainingProjection};

use crate::core::{AccumulatorConfig, EngineError};
use crate::evaluation::{Batch, BatchResult, EvalBatchOptions, Evaluator};
use crate::utils::domain::Domain;

pub struct GammaLoopEvaluator;

impl GammaLoopEvaluator {
    pub fn from_params(_params: GammaLoopParams) -> Result<Self, EngineError> {
        Err(gammaloop_disabled_error())
    }

    pub fn resolve_domain_from_params(_params: GammaLoopParams) -> Result<Domain, EngineError> {
        Err(gammaloop_disabled_error())
    }
}

impl Evaluator for GammaLoopEvaluator {
    fn domain(&self) -> Domain {
        Domain::continuous(0)
    }

    fn eval_batch(
        &mut self,
        _batch: &Batch,
        _accumulator: &AccumulatorConfig,
        _options: EvalBatchOptions,
    ) -> Result<BatchResult, EngineError> {
        Err(EngineError::eval(
            "evaluator.kind = \"gammaloop\" requires a gammaboard build with the default \"gammaloop\" feature enabled",
        ))
    }
}

fn gammaloop_disabled_error() -> EngineError {
    EngineError::invalid_input(
        "evaluator.kind = \"gammaloop\" requires a gammaboard build with the default \"gammaloop\" feature enabled",
    )
}
