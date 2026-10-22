use super::{Batch, BatchResult};
use crate::core::{AccumulatorConfig, EngineError};
use crate::sampling::LatentBatch;
use crate::utils::domain::Domain;
use serde_json::{Value as JsonValue, json};

#[derive(Debug, Clone, Copy, Default)]
pub struct EvalBatchOptions<'a> {
    pub require_training_values: bool,
    pub stratification: Option<(&'a crate::core::StratificationPlan, &'a [usize])>,
}

impl EvalBatchOptions<'_> {
    pub fn matches(&self, index: usize, values: &[f64]) -> Result<bool, EngineError> {
        match self.stratification {
            Some((plan, labels)) => plan.matches(
                *labels
                    .get(index)
                    .ok_or_else(|| EngineError::eval("missing stratum label"))?,
                values,
            ),
            None => Ok(true),
        }
    }

    pub fn validate(&self, samples: usize, components: &[String]) -> Result<(), EngineError> {
        if let Some((plan, labels)) = self.stratification {
            plan.validate()?;
            if labels.len() != samples || plan.components != components {
                return Err(EngineError::eval("stratification batch layout mismatch"));
            }
        }
        Ok(())
    }
}

pub trait Evaluator: Send {
    fn domain(&self) -> Domain;
    fn metadata(&self) -> JsonValue {
        json!({})
    }

    fn diagnostics(&mut self) -> JsonValue {
        json!({})
    }

    /// Unweighted, sample-major values used by runtime-owned evaluation modifiers.
    fn eval_values(
        &mut self,
        _batch: &Batch,
        _components: &[String],
    ) -> Result<Vec<f64>, EngineError> {
        Err(EngineError::invalid_input(
            "this evaluator does not expose per-sample values for stratification",
        ))
    }

    fn eval_batch(
        &mut self,
        batch: &Batch,
        accumulator: &AccumulatorConfig,
        options: EvalBatchOptions,
    ) -> Result<BatchResult, EngineError> {
        let components = accumulator
            .components()
            .unwrap_or(&["value".into()])
            .to_vec();
        let values = self.eval_values(batch, &components)?;
        super::stratification::accumulate_values(batch, &components, &values, accumulator, options)
    }
}

pub trait Materializer: Send {
    fn validate_domain(&self, _domain: &Domain) -> Result<(), EngineError> {
        Ok(())
    }

    fn materialize_batch(&mut self, latent_batch: &LatentBatch) -> Result<Batch, EngineError>;
}

pub trait BatchTransform: Send {
    fn validate_domain(&self, _domain: &Domain) -> Result<(), EngineError> {
        Ok(())
    }

    fn apply(&mut self, batch: Batch) -> Result<Batch, EngineError>;
}
