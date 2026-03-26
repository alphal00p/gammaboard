use crate::core::{BuildError, EngineError, SamplerAggregatorConfig};
use crate::evaluation::PointSpec;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use super::{LatentBatchSpec, SamplePlan};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SamplerAggregatorSnapshot {
    NaiveMonteCarlo { raw: JsonValue },
    RasterPlane { raw: JsonValue },
    RasterLine { raw: JsonValue },
    HavanaTraining { raw: JsonValue },
    HavanaInference { raw: JsonValue },
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
                SamplerAggregatorSnapshot::HavanaTraining { .. },
                SamplerAggregatorConfig::HavanaTraining { .. }
            ) | (
                SamplerAggregatorSnapshot::HavanaInference { .. },
                SamplerAggregatorConfig::HavanaInference { .. }
            )
        )
    }
}

pub trait SamplerAggregator: Send {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn training_samples_remaining(&self) -> Option<usize> {
        None
    }
    fn sample_plan(&mut self) -> Result<SamplePlan, EngineError> {
        Ok(SamplePlan::Produce {
            nr_samples: usize::MAX,
        })
    }
    fn produce_latent_batch(&mut self, nr_samples: usize) -> Result<LatentBatchSpec, EngineError>;
    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError>;
    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError>;
    fn get_diagnostics(&mut self) -> JsonValue {
        json!("{}")
    }
}
