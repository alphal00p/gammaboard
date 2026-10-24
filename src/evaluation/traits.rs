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
    pub fn stratum_assignment(
        &self,
        index: usize,
        values: &[f64],
    ) -> Result<Option<super::batch::StratumPair<'_>>, EngineError> {
        self.stratification
            .map(|(plan, labels)| {
                let requested = *labels
                    .get(index)
                    .ok_or_else(|| EngineError::eval("missing stratum assignment"))?;
                let actual = plan.partition.classify(values)?;
                Ok((
                    plan.partition.prefix(requested),
                    plan.partition.prefix(actual),
                ))
            })
            .transpose()
    }

    pub fn partition_diagnostics(
        &self,
        raw: &[f64],
        width: usize,
    ) -> Result<Option<super::accumulator::PartitionDiagnostics>, EngineError> {
        self.stratification
            .map(|(plan, labels)| {
                if width == 0 || raw.len() != width * labels.len() {
                    return Err(EngineError::eval("partition diagnostic layout mismatch"));
                }
                let mut diagnostics = super::accumulator::PartitionDiagnostics {
                    partition: plan.partition.clone(),
                    attempts: vec![0; plan.partition.centers().len()],
                    matches: vec![0; plan.partition.centers().len()],
                    feedback_samples: if self.require_training_values {
                        labels.len() as u64
                    } else {
                        0
                    },
                };
                for (&requested, values) in labels.iter().zip(raw.chunks_exact(width)) {
                    diagnostics.attempts[requested] += 1;
                    diagnostics.matches[requested] +=
                        u64::from(plan.partition.classify(values)? == requested);
                }
                diagnostics.validate(labels.len() as i64)?;
                Ok(diagnostics)
            })
            .transpose()
    }

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

    pub fn feedback(
        &self,
        unmasked_values: Option<Vec<f64>>,
        raw: &[f64],
        width: usize,
    ) -> Result<Option<crate::sampling::Feedback>, EngineError> {
        unmasked_values
            .map(|values| {
                let mut feedback = crate::sampling::Feedback::new(values);
                if let Some((plan, labels)) = self.stratification {
                    if width == 0
                        || raw.len() != labels.len() * width
                        || feedback.len() != labels.len()
                    {
                        return Err(EngineError::eval(
                            "training feedback/stratum sample count mismatch",
                        ));
                    }
                    feedback.strata = Some(crate::sampling::StratumAssignments {
                        prefixes: plan.partition.prefixes().cloned().collect(),
                        requested: labels.to_vec(),
                        actual: raw
                            .chunks_exact(width)
                            .map(|row| plan.partition.classify(row))
                            .collect::<Result<_, _>>()?,
                    });
                }
                feedback.validate()?;
                Ok(feedback)
            })
            .transpose()
    }

    pub fn validate(&self, samples: usize, components: &[String]) -> Result<(), EngineError> {
        if let Some((plan, labels)) = self.stratification {
            plan.validate()?;
            if labels.len() != samples
                || plan.components != components
                || labels.iter().any(|&k| k >= plan.partition.centers().len())
            {
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
