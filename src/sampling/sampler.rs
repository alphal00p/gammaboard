use crate::core::{EngineError, SamplingConfig};
use crate::utils::domain::Domain;
use serde::{Deserialize, Serialize};
use serde_json::{Value as JsonValue, json};
use std::collections::BTreeMap;

use super::LatentBatch;

pub type PdfPoint = (Vec<i64>, Vec<f64>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscreteSubspace {
    pub fixed_dims: BTreeMap<usize, i64>,
}

/// Runtime-owned construction identity plus opaque, implementation-owned state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSampler {
    pub version: u32,
    pub config: SamplingConfig,
    pub state: JsonValue,
}

impl SavedSampler {
    pub fn contains_havana_grid(&self) -> bool {
        self.validate().is_ok()
            && matches!(
                self.config,
                SamplingConfig {
                    sampler: crate::core::SamplerConfig::HavanaTraining { .. },
                    ..
                } | SamplingConfig {
                    sampler: crate::core::SamplerConfig::HavanaInference { .. },
                    ..
                }
            )
            && self.state.get("grid").is_some()
    }

    pub fn restore(
        self,
        domain: &Domain,
        evaluator_metadata: JsonValue,
    ) -> Result<Box<dyn Sampler>, EngineError> {
        self.validate()?;
        self.config
            .restore_state(self.state, domain, evaluator_metadata)
    }

    pub(crate) fn validate(&self) -> Result<(), EngineError> {
        if self.version != crate::compatibility::MODEL_VERSION || self.state.is_null() {
            return Err(EngineError::build(
                "unsupported or empty saved sampler state",
            ));
        }
        Ok(())
    }
}

pub trait Sampler: Send {
    fn validate_domain(&self, domain: &Domain) -> Result<(), EngineError>;
    /// Generate at most `max_samples` samples (a positive per-draw limit).
    /// The runtime caps this by queue tuning, the remaining task budget, and
    /// any durable campaign allowance. See docs/execution-model.md.
    /// Draws may be smaller, e.g. at a training boundary, but never empty.
    /// The limit can change between draws without changing feedback boundaries.
    fn generate(&mut self, max_samples: usize) -> Result<Generation, EngineError>;
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
    /// Non-null JSON containing all private state needed for deterministic restore,
    /// including pending feedback. The runtime records construction config separately.
    fn snapshot(&mut self) -> Result<JsonValue, EngineError>;
    fn diagnostics(&mut self) -> JsonValue {
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
        batch: LatentBatch,
        training_remaining: Option<usize>,
    },
    Waiting,
    Finished,
}

impl Generation {
    pub fn batch(batch: LatentBatch, training_remaining: Option<usize>) -> Self {
        Self::Batch {
            batch,
            training_remaining,
        }
    }

    /// Unwrap the flat queue payload without materializing evaluator `Point`s.
    pub fn into_batch(self) -> Result<LatentBatch, EngineError> {
        match self {
            Self::Batch { batch, .. } => Ok(batch),
            Self::Waiting => Err(EngineError::engine("sampler is waiting for feedback")),
            Self::Finished => Err(EngineError::engine("sampler has finished")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_sampler_roundtrip_preserves_next_draw_and_rejects_invalid_envelopes() {
        let config = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                params: Default::default(),
            },
            materializer: None,
            transforms: None,
        };
        let domain = Domain::continuous(2);
        let mut sampler = config.create(domain.clone(), None, json!({})).unwrap();
        sampler.generate(7).unwrap();
        let saved = config.snapshot(&mut *sampler).unwrap();
        let json = serde_json::to_value(&saved).unwrap();
        assert!(json["state"]["rng"].is_object());
        assert!(json.get("raw").is_none());
        let mut restored = serde_json::from_value::<SavedSampler>(json)
            .unwrap()
            .restore(&domain, json!({}))
            .unwrap();
        assert_eq!(
            sampler.generate(13).unwrap(),
            restored.generate(13).unwrap()
        );
        let mut invalid = saved.clone();
        invalid.version += 1;
        assert!(invalid.restore(&domain, json!({})).is_err());
        let mut invalid = saved;
        invalid.state = JsonValue::Null;
        assert!(invalid.restore(&domain, json!({})).is_err());
    }
    #[test]
    fn fresh_configuration_and_pending_feedback_restore_are_distinct() {
        let config = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                params: crate::sampling::NaiveMonteCarloSamplerParams {
                    seed: 19,
                    training_window_samples: 7,
                    ..Default::default()
                },
            },
            materializer: None,
            transforms: None,
        };
        let domain = Domain::continuous(2);
        let mut original = config.create(domain.clone(), None, json!({})).unwrap();
        original.generate(7).unwrap();
        let saved = config.snapshot(&mut *original).unwrap();
        assert!(saved.state.get("params").is_none());
        let mut restored = saved.clone().restore(&domain, json!({})).unwrap();
        assert_eq!(restored.generate(5).unwrap(), Generation::Waiting);
        original.feedback(&[1.; 7]).unwrap();
        restored.feedback(&[1.; 7]).unwrap();
        assert_eq!(original.generate(5).unwrap(), restored.generate(5).unwrap());
        let changed = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::NaiveMonteCarlo {
                params: crate::sampling::NaiveMonteCarloSamplerParams {
                    seed: 27,
                    ..Default::default()
                },
            },
            materializer: None,
            transforms: None,
        };
        let mut fresh = changed.create(domain.clone(), None, json!({})).unwrap();
        let mut reference = changed.create(domain.clone(), None, json!({})).unwrap();
        assert_eq!(fresh.generate(5).unwrap(), reference.generate(5).unwrap());
        let mut old_seed = config.create(domain.clone(), None, json!({})).unwrap();
        let mut new_seed = changed.create(domain.clone(), None, json!({})).unwrap();
        assert_ne!(new_seed.generate(5).unwrap(), old_seed.generate(5).unwrap());
        assert!(
            changed
                .from_stage(
                    &domain,
                    super::super::StageHandoff {
                        sampler_snapshot: Some(&saved),
                        observable_state: None,
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn havana_handoff_applies_new_seed_and_resets_stage_counters() {
        use crate::sampling::{HavanaInferenceSamplerParams, HavanaSamplerParams, StageHandoff};
        let domain = Domain::continuous(2);
        let training = SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::HavanaTraining {
                params: HavanaSamplerParams {
                    seed: 3,
                    ..Default::default()
                },
            },
            materializer: None,
            transforms: None,
        };
        let mut sampler = training
            .create(domain.clone(), Some(32), json!({}))
            .unwrap();
        let source = training.snapshot(&mut *sampler).unwrap();
        assert!(source.state.get("samples_for_update").is_none());
        let inference = |seed| SamplingConfig {
            stratification: None,
            sampler: crate::core::SamplerConfig::HavanaInference {
                params: HavanaInferenceSamplerParams { seed: Some(seed) },
            },
            materializer: None,
            transforms: None,
        };
        let mut first = inference(7)
            .from_stage(
                &domain,
                StageHandoff {
                    sampler_snapshot: Some(&source),
                    observable_state: None,
                },
            )
            .unwrap();
        first.generate(11).unwrap();
        let saved = inference(7).snapshot(&mut *first).unwrap();
        let mut restored = saved.clone().restore(&domain, json!({})).unwrap();
        assert_eq!(first.generate(5).unwrap(), restored.generate(5).unwrap());
        let mut handed = inference(29)
            .from_stage(
                &domain,
                StageHandoff {
                    sampler_snapshot: Some(&saved),
                    observable_state: None,
                },
            )
            .unwrap();
        let mut reference = inference(29)
            .from_stage(
                &domain,
                StageHandoff {
                    sampler_snapshot: Some(&source),
                    observable_state: None,
                },
            )
            .unwrap();
        assert_eq!(handed.snapshot().unwrap()["samples_produced"], 0);
        assert_eq!(handed.generate(5).unwrap(), reference.generate(5).unwrap());
    }
}
