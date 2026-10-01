use crate::core::{BuildError, EngineError, SamplerAggregatorConfig};
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeMap;

use super::LatentBatchSpec;

pub type PdfPoint = (Vec<i64>, Vec<f64>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscreteSubspace {
    pub fixed_dims: BTreeMap<usize, i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SamplerAggregatorSnapshot {
    NaiveMonteCarlo { raw: JsonValue },
    RasterPlane { raw: JsonValue },
    RasterLine { raw: JsonValue },
    PdfAdaptationRasterPlane { raw: JsonValue },
    PdfAdaptationRasterLine { raw: JsonValue },
    HavanaTraining { raw: JsonValue },
    HavanaInference { raw: JsonValue },
    ProcessSampler { raw: JsonValue },
}

impl SamplerAggregatorSnapshot {
    pub fn matches_config(&self, config: &SamplerAggregatorConfig) -> bool {
        matches!(
            (self, config),
            (
                SamplerAggregatorSnapshot::NaiveMonteCarlo { .. },
                SamplerAggregatorConfig::NaiveMonteCarlo { .. }
            ) | (
                SamplerAggregatorSnapshot::RasterPlane { .. },
                SamplerAggregatorConfig::RasterPlane { .. }
            ) | (
                SamplerAggregatorSnapshot::RasterLine { .. },
                SamplerAggregatorConfig::RasterLine { .. }
            ) | (
                SamplerAggregatorSnapshot::PdfAdaptationRasterPlane { .. },
                SamplerAggregatorConfig::PdfAdaptationRasterPlane { .. }
            ) | (
                SamplerAggregatorSnapshot::PdfAdaptationRasterLine { .. },
                SamplerAggregatorConfig::PdfAdaptationRasterLine { .. }
            ) | (
                SamplerAggregatorSnapshot::HavanaTraining { .. },
                SamplerAggregatorConfig::HavanaTraining { .. }
            ) | (
                SamplerAggregatorSnapshot::HavanaInference { .. },
                SamplerAggregatorConfig::HavanaInference { .. }
            ) | (
                SamplerAggregatorSnapshot::ProcessSampler { .. },
                SamplerAggregatorConfig::ProcessSampler { .. }
            )
        )
    }

    pub fn contains_havana_grid(&self) -> bool {
        match self {
            SamplerAggregatorSnapshot::HavanaTraining { raw }
            | SamplerAggregatorSnapshot::HavanaInference { raw } => raw.get("grid").is_some(),
            _ => false,
        }
    }
}

pub trait SamplerAggregator: Send {
    fn validate_domain(&self, domain: &Domain) -> Result<(), BuildError>;
    /// Generate a sampler-sized draw, bounded only by the remaining task budget.
    /// `None` means the task has no sample-count limit. Never return an empty draw.
    fn generate(
        &mut self,
        remaining_sample_budget: Option<usize>,
    ) -> Result<Generation, EngineError>;
    /// One weighted scalar per sample of the oldest feedback-bearing draw.
    /// Calls preserve generation order and never split or combine draws.
    fn feedback(&mut self, values: &[f64]) -> Result<(), EngineError>;
    fn pdf_batch(&mut self, points: &[PdfPoint]) -> Result<Vec<Option<f64>>, EngineError> {
        Ok(vec![None; points.len()])
    }
    fn discrete_pdf_batch(
        &mut self,
        subspaces: &[DiscreteSubspace],
    ) -> Result<Vec<Option<f64>>, EngineError> {
        Ok(vec![None; subspaces.len()])
    }
    fn global_pdf_norm(&mut self) -> Result<f64, EngineError> {
        Ok(1.0)
    }
    fn persisted_output(&mut self) -> Result<Option<JsonValue>, EngineError> {
        Ok(None)
    }
    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError>;
    fn get_diagnostics(&mut self) -> JsonValue {
        json!({})
    }
}

/// Generation and evaluator batch sizes are independent. A finite training
/// window is reported before this draw, allowing fair evaluator partitioning.
#[derive(Debug, Clone, PartialEq)]
// The payload is the common case and is consumed immediately; keep it inline
// rather than allocating a separate box for each generated batch.
#[allow(clippy::large_enum_variant)]
pub enum Generation {
    Batch {
        batch: LatentBatchSpec,
        training_remaining: Option<usize>,
    },
    Waiting,
    Finished,
}

impl Generation {
    pub fn batch(batch: LatentBatchSpec, training_remaining: Option<usize>) -> Self {
        Self::Batch {
            batch,
            training_remaining,
        }
    }

    /// Unwrap the flat queue payload without materializing evaluator `Point`s.
    pub fn into_batch(self) -> Result<LatentBatchSpec, EngineError> {
        match self {
            Self::Batch { batch, .. } => Ok(batch),
            Self::Waiting => Err(EngineError::engine("sampler is waiting for feedback")),
            Self::Finished => Err(EngineError::engine("sampler has finished")),
        }
    }
}

pub(crate) const fn default_generation_batch_size() -> usize {
    1_048_576
}
