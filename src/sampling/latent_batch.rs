//! Latent batch abstraction for sampler-owned queue payloads.

use bincode::config::{Configuration, standard};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

use crate::core::AccumulatorConfig;
use crate::evaluation::{Batch, BatchError, Point};
use crate::utils::rng::SerializableMonteCarloRng;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentBatch {
    pub nr_samples: usize,
    pub accumulator: AccumulatorConfig,
    pub payload: LatentBatchPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentBatchSpec {
    pub nr_samples: usize,
    pub accumulator: AccumulatorConfig,
    pub payload: LatentBatchPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LatentBatchPayload {
    IndexedBatch {
        discrete_signatures: Vec<Vec<i64>>,
        discrete_map: Vec<usize>,
        continuous_layouts: Vec<usize>,
        continuous_values: Vec<f64>,
        weights: Vec<f64>,
    },
    HavanaInference {
        rng_state: SerializableMonteCarloRng,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SamplePlan {
    Produce { nr_samples: usize },
    Pause,
}

#[derive(Debug, Serialize, Deserialize)]
struct LatentBatchBinary {
    nr_samples: usize,
    accumulator: AccumulatorConfig,
    payload: LatentBatchPayloadBinary,
}

#[derive(Debug, Serialize, Deserialize)]
enum LatentBatchPayloadBinary {
    IndexedBatch {
        discrete_signatures: Vec<Vec<i64>>,
        discrete_map: Vec<usize>,
        continuous_layouts: Vec<usize>,
        continuous_values: Vec<f64>,
        weights: Vec<f64>,
    },
    HavanaInference {
        rng_state: SerializableMonteCarloRng,
    },
}

impl LatentBatchPayload {
    pub fn from_batch(batch: &Batch) -> Self {
        let mut discrete_signatures = Vec::<Vec<i64>>::new();
        let mut discrete_index = HashMap::<Vec<i64>, usize>::new();
        let mut discrete_map = Vec::with_capacity(batch.size());
        let mut continuous_layouts = Vec::with_capacity(batch.size());
        let mut continuous_values = Vec::new();
        let mut weights = Vec::with_capacity(batch.size());

        for point in batch.points() {
            let signature_idx = if let Some(&idx) = discrete_index.get(&point.discrete) {
                idx
            } else {
                let idx = discrete_signatures.len();
                let signature = point.discrete.clone();
                discrete_index.insert(signature.clone(), idx);
                discrete_signatures.push(signature);
                idx
            };
            discrete_map.push(signature_idx);
            continuous_layouts.push(point.continuous.len());
            continuous_values.extend_from_slice(&point.continuous);
            let sampler_weight = point
                .factor_value("sampler_weight")
                .expect("batch point missing sampler_weight factor");
            weights.push(sampler_weight);
        }

        Self::IndexedBatch {
            discrete_signatures,
            discrete_map,
            continuous_layouts,
            continuous_values,
            weights,
        }
    }

    pub fn into_batch(self) -> Result<Batch, BatchError> {
        match self {
            Self::IndexedBatch {
                discrete_signatures,
                discrete_map,
                continuous_layouts,
                continuous_values,
                weights,
            } => decode_indexed_batch(
                &discrete_signatures,
                &discrete_map,
                &continuous_layouts,
                &continuous_values,
                &weights,
            ),
            Self::HavanaInference { .. } => Err(BatchError::layout(
                "havana_inference latent payload must be materialized by a materializer",
            )),
        }
    }

    pub fn as_batch(&self) -> Result<Batch, BatchError> {
        match self {
            Self::IndexedBatch {
                discrete_signatures,
                discrete_map,
                continuous_layouts,
                continuous_values,
                weights,
            } => decode_indexed_batch(
                discrete_signatures,
                discrete_map,
                continuous_layouts,
                continuous_values,
                weights,
            ),
            Self::HavanaInference { .. } => Err(BatchError::layout(
                "havana_inference latent payload must be materialized by a materializer",
            )),
        }
    }
}

impl LatentBatchSpec {
    pub fn from_batch(batch: &Batch) -> Self {
        Self {
            nr_samples: batch.size(),
            accumulator: AccumulatorConfig::scalar(),
            payload: LatentBatchPayload::from_batch(batch),
        }
    }

    pub fn build(self) -> LatentBatch {
        LatentBatch {
            nr_samples: self.nr_samples,
            accumulator: self.accumulator,
            payload: self.payload,
        }
    }

    pub fn with_accumulator_config(mut self, accumulator: AccumulatorConfig) -> Self {
        self.accumulator = accumulator;
        self
    }
}

impl LatentBatch {
    fn binary_config() -> Configuration {
        standard()
    }

    pub fn validate_nr_samples(&self) -> Result<(), BatchError> {
        if self.nr_samples == 0 {
            return Err(BatchError::layout(
                "latent batch nr_samples must be greater than zero",
            ));
        }
        match &self.payload {
            LatentBatchPayload::IndexedBatch {
                discrete_map,
                continuous_layouts,
                continuous_values,
                weights,
                ..
            } => {
                if weights.len() != self.nr_samples {
                    return Err(BatchError::layout(format!(
                        "latent batch nr_samples mismatch: nr_samples={}, weights={}",
                        self.nr_samples,
                        weights.len()
                    )));
                }
                if discrete_map.len() != self.nr_samples
                    || continuous_layouts.len() != self.nr_samples
                {
                    return Err(BatchError::layout(format!(
                        "latent batch indexed shape mismatch: nr_samples={}, discrete_map={}, continuous_layouts={}",
                        self.nr_samples,
                        discrete_map.len(),
                        continuous_layouts.len()
                    )));
                }
                let expected_continuous_values = continuous_layouts.iter().copied().sum::<usize>();
                if continuous_values.len() != expected_continuous_values {
                    return Err(BatchError::layout(format!(
                        "latent batch continuous payload mismatch: expected={}, actual={}",
                        expected_continuous_values,
                        continuous_values.len()
                    )));
                }
            }
            LatentBatchPayload::HavanaInference { .. } => {}
        }
        Ok(())
    }

    pub fn into_json(&self) -> JsonValue {
        serde_json::to_value(self).expect("LatentBatch serialization should never fail")
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, BatchError> {
        let latent: Self = serde_json::from_value(value.clone())?;
        latent.validate_nr_samples()?;
        Ok(latent)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, BatchError> {
        let payload = match &self.payload {
            LatentBatchPayload::IndexedBatch {
                discrete_signatures,
                discrete_map,
                continuous_layouts,
                continuous_values,
                weights,
            } => LatentBatchPayloadBinary::IndexedBatch {
                discrete_signatures: discrete_signatures.clone(),
                discrete_map: discrete_map.clone(),
                continuous_layouts: continuous_layouts.clone(),
                continuous_values: continuous_values.clone(),
                weights: weights.clone(),
            },
            LatentBatchPayload::HavanaInference { rng_state } => {
                LatentBatchPayloadBinary::HavanaInference {
                    rng_state: rng_state.clone(),
                }
            }
        };
        bincode::serde::encode_to_vec(
            LatentBatchBinary {
                nr_samples: self.nr_samples,
                accumulator: self.accumulator.clone(),
                payload,
            },
            Self::binary_config(),
        )
        .map_err(|err| BatchError::layout(format!("invalid latent batch payload: {err}")))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BatchError> {
        let (latent, _): (LatentBatchBinary, usize) =
            bincode::serde::decode_from_slice(bytes, Self::binary_config()).map_err(|err| {
                BatchError::layout(format!("invalid latent batch payload: {err}"))
            })?;
        let payload = match latent.payload {
            LatentBatchPayloadBinary::IndexedBatch {
                discrete_signatures,
                discrete_map,
                continuous_layouts,
                continuous_values,
                weights,
            } => LatentBatchPayload::IndexedBatch {
                discrete_signatures,
                discrete_map,
                continuous_layouts,
                continuous_values,
                weights,
            },
            LatentBatchPayloadBinary::HavanaInference { rng_state } => {
                LatentBatchPayload::HavanaInference { rng_state }
            }
        };
        let restored = Self {
            nr_samples: latent.nr_samples,
            accumulator: latent.accumulator,
            payload,
        };
        restored.validate_nr_samples()?;
        Ok(restored)
    }
}

fn decode_indexed_batch(
    discrete_signatures: &[Vec<i64>],
    discrete_map: &[usize],
    continuous_layouts: &[usize],
    continuous_values: &[f64],
    weights: &[f64],
) -> Result<Batch, BatchError> {
    let nr_samples = weights.len();
    if discrete_map.len() != nr_samples || continuous_layouts.len() != nr_samples {
        return Err(BatchError::layout(format!(
            "indexed latent batch shape mismatch: discrete_map={}, continuous_layouts={}, weights={nr_samples}",
            discrete_map.len(),
            continuous_layouts.len(),
        )));
    }

    let mut continuous_offset = 0usize;
    let mut points = Vec::with_capacity(nr_samples);
    for sample_idx in 0..nr_samples {
        let signature_idx = discrete_map[sample_idx];
        let discrete = discrete_signatures.get(signature_idx).ok_or_else(|| {
            BatchError::layout(format!(
                "indexed latent batch discrete_map[{sample_idx}] points to missing signature {signature_idx}"
            ))
        })?;
        let continuous_len = continuous_layouts[sample_idx];
        let next_continuous_offset = continuous_offset
            .checked_add(continuous_len)
            .ok_or_else(|| BatchError::layout("indexed latent batch continuous offset overflow"))?;
        let continuous = continuous_values
            .get(continuous_offset..next_continuous_offset)
            .ok_or_else(|| {
                BatchError::layout(format!(
                    "indexed latent batch continuous values too short for sample {sample_idx}"
                ))
            })?;
        points.push(Point::new(
            continuous.to_vec(),
            discrete.clone(),
            weights[sample_idx],
        ));
        continuous_offset = next_continuous_offset;
    }

    if continuous_offset != continuous_values.len() {
        return Err(BatchError::layout(format!(
            "indexed latent batch continuous values have trailing data: consumed={continuous_offset} total={}",
            continuous_values.len()
        )));
    }

    Batch::new(points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::Point;

    #[test]
    fn latent_batch_roundtrips_batch_payload() {
        let batch = Batch::from_points([
            Point::new(vec![0.5], Vec::new(), 1.0),
            Point::new(vec![1.5], Vec::new(), 1.0),
        ])
        .expect("batch creation");
        let latent = LatentBatchSpec::from_batch(&batch).build();
        let json = latent.into_json();
        let restored = LatentBatch::from_json(&json).expect("latent batch");
        assert_eq!(restored.nr_samples, 2);
        let restored_batch = restored.payload.as_batch().expect("batch payload");
        assert_eq!(restored_batch, batch);
    }

    #[test]
    fn latent_batch_roundtrips_binary_payload() {
        let batch = Batch::from_points([
            Point::new(vec![0.5], Vec::new(), 1.0),
            Point::new(vec![1.5], Vec::new(), 1.0),
        ])
        .expect("batch creation");
        let latent = LatentBatchSpec::from_batch(&batch).build();
        let bytes = latent.to_bytes().expect("latent batch bytes");
        let restored = LatentBatch::from_bytes(&bytes).expect("latent batch");
        assert_eq!(restored, latent);
    }

    #[test]
    fn latent_batch_roundtrips_heterogeneous_batch_payload() {
        let batch = Batch::from_points([
            Point::new(vec![0.5, 1.5], vec![1, 2], 1.0),
            Point::new(vec![2.5], vec![1, 2], 2.0),
            Point::new(Vec::new(), vec![9], 3.0),
            Point::new(vec![4.5, 5.5, 6.5], Vec::new(), 4.0),
        ])
        .expect("batch creation");
        let latent = LatentBatchSpec::from_batch(&batch).build();

        let json = latent.into_json();
        let restored = LatentBatch::from_json(&json).expect("latent from json");
        let restored_batch = restored.payload.into_batch().expect("batch payload");

        assert_eq!(restored_batch, batch);
    }

    #[test]
    fn latent_batch_deduplicates_discrete_signatures() {
        let batch = Batch::from_points([
            Point::new(vec![0.5], vec![1, 2], 1.0),
            Point::new(vec![1.5, 2.5], vec![1, 2], 2.0),
            Point::new(vec![3.5], vec![7], 3.0),
            Point::new(vec![4.5], vec![1, 2], 4.0),
        ])
        .expect("batch creation");
        let latent = LatentBatchSpec::from_batch(&batch).build();

        let LatentBatchPayload::IndexedBatch {
            discrete_signatures,
            discrete_map,
            continuous_layouts,
            continuous_values,
            weights,
        } = &latent.payload
        else {
            panic!("expected indexed batch payload");
        };

        assert_eq!(discrete_signatures, &vec![vec![1, 2], vec![7]]);
        assert_eq!(discrete_map, &vec![0, 0, 1, 0]);
        assert_eq!(continuous_layouts, &vec![1, 2, 1, 1]);
        assert_eq!(continuous_values, &vec![0.5, 1.5, 2.5, 3.5, 4.5]);
        assert_eq!(weights, &vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn latent_batch_rejects_mismatched_nr_samples() {
        let latent = LatentBatch {
            nr_samples: 2,
            accumulator: AccumulatorConfig::scalar(),
            payload: LatentBatchPayload::IndexedBatch {
                discrete_signatures: vec![vec![1]],
                discrete_map: vec![0],
                continuous_layouts: vec![1],
                continuous_values: vec![0.5],
                weights: vec![1.0],
            },
        };

        let err = latent.validate_nr_samples().expect_err("expected mismatch");
        assert!(err.to_string().contains("nr_samples mismatch"));
    }
}
