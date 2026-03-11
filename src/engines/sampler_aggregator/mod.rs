mod havana;
mod naive_monte_carlo;

use super::{BuildError, EngineError};
use crate::core::{Batch, PointSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};

use self::havana::{HavanaSampler, HavanaSamplerParams, HavanaSamplerSnapshot};
use self::naive_monte_carlo::{NaiveMonteCarloSamplerAggregator, NaiveMonteCarloSamplerParams};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SamplerAggregatorSnapshot {
    NaiveMonteCarlo {
        state: NaiveMonteCarloSamplerAggregator,
    },
    Havana {
        raw: JsonValue,
    },
}

impl SamplerAggregatorSnapshot {
    pub fn into_runtime(
        self,
        point_spec: &PointSpec,
    ) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { state } => Ok(Box::new(
                NaiveMonteCarloSamplerAggregator::from_snapshot(state, point_spec)?,
            )),
            Self::Havana { raw } => {
                let snapshot: HavanaSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(HavanaSampler::from_snapshot(
                    snapshot, point_spec,
                )?))
            }
        }
    }
}

/// Owns adaptive sampling training for a single run.
pub trait SamplerAggregator: Send {
    fn validate_point_spec(&self, point_spec: &PointSpec) -> Result<(), BuildError>;
    fn get_init_metadata(&mut self) -> JsonValue {
        json!({})
    }
    fn is_training_active(&self) -> bool {
        true
    }
    fn get_max_samples(&self) -> Option<usize> {
        None
    }
    fn produce_batch(&mut self, nr_samples: usize) -> Result<Batch, EngineError>;
    fn ingest_training_weights(&mut self, training_weights: &[f64]) -> Result<(), EngineError>;
    fn snapshot(&mut self) -> Result<SamplerAggregatorSnapshot, EngineError>;
    fn get_diagnostics(&mut self) -> JsonValue {
        json!("{}")
    }
}

impl crate::engines::SamplerAggregatorConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::NaiveMonteCarlo { .. } => "naive_monte_carlo",
            Self::Havana { .. } => "havana",
        }
    }

    pub fn build(&self, point_spec: PointSpec) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { params } => {
                let params: NaiveMonteCarloSamplerParams =
                    serde_json::from_value(JsonValue::Object(params.clone()))
                        .map_err(|err| BuildError::invalid_input(err.to_string()))?;
                Ok(Box::new(
                    NaiveMonteCarloSamplerAggregator::from_params_and_point_spec(
                        params,
                        &point_spec,
                    )?,
                ))
            }
            Self::Havana { params } => {
                let params: HavanaSamplerParams =
                    serde_json::from_value(JsonValue::Object(params.clone()))
                        .map_err(|err| BuildError::invalid_input(err.to_string()))?;
                Ok(Box::new(HavanaSampler::from_params_and_point_spec(
                    params,
                    &point_spec,
                )?))
            }
        }
    }
}
