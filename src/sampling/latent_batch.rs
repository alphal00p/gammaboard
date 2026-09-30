//! Latent batch abstraction for sampler-owned queue payloads.

use bincode::config::{Configuration, standard};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::{borrow::Cow, collections::HashMap};

use crate::core::AccumulatorConfig;
use crate::evaluation::{Batch, BatchError, Point};
use crate::utils::rng::SerializableMonteCarloRng;

/// Bounds replay at arbitrary split points without materializing coordinates.
pub(crate) const RNG_CHECKPOINT_STRIDE: usize = 1024;

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
    HavanaInferenceIndexed {
        rng_states: Vec<SerializableMonteCarloRng>,
        offset: usize,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct LatentBatchBinary<'a> {
    nr_samples: usize,
    accumulator: Cow<'a, AccumulatorConfig>,
    payload: LatentBatchPayloadBinary<'a>,
}

#[derive(Debug, Serialize, Deserialize)]
enum LatentBatchPayloadBinary<'a> {
    IndexedBatch {
        discrete_signatures: Cow<'a, [Vec<i64>]>,
        discrete_map: Cow<'a, [usize]>,
        continuous_layouts: Cow<'a, [usize]>,
        continuous_values: Cow<'a, [f64]>,
        weights: Cow<'a, [f64]>,
    },
    HavanaInference {
        rng_state: Cow<'a, SerializableMonteCarloRng>,
    },
    HavanaInferenceIndexed {
        rng_states: Cow<'a, [SerializableMonteCarloRng]>,
        offset: usize,
    },
}

/// Builds the queue layout without allocating evaluator-side `Point`s.
pub(crate) struct IndexedBatchBuilder {
    discrete_signatures: Vec<Vec<i64>>,
    discrete_index: HashMap<Vec<i64>, usize>,
    empty_signature: Option<usize>,
    discrete_map: Vec<usize>,
    continuous_layouts: Vec<usize>,
    continuous_values: Vec<f64>,
    weights: Vec<f64>,
}

impl IndexedBatchBuilder {
    pub(crate) fn new(nr_samples: usize) -> Self {
        Self {
            discrete_signatures: Vec::new(),
            discrete_index: HashMap::new(),
            empty_signature: None,
            discrete_map: Vec::with_capacity(nr_samples),
            continuous_layouts: Vec::with_capacity(nr_samples),
            continuous_values: Vec::new(),
            weights: Vec::with_capacity(nr_samples),
        }
    }

    pub(crate) fn push(&mut self, discrete: &[i64], continuous: &[f64], weight: f64) {
        if self.weights.is_empty() {
            // Exact for uniform layouts; ragged batches can grow as needed.
            self.continuous_values
                .reserve(continuous.len().saturating_mul(self.weights.capacity()));
        }
        // Continuous-only samples need neither hashing nor a slice comparison.
        let signature_idx = if discrete.is_empty() {
            *self.empty_signature.get_or_insert_with(|| {
                let idx = self.discrete_signatures.len();
                self.discrete_signatures.push(Vec::new());
                idx
            })
        } else if let Some(&idx) = self.discrete_index.get(discrete) {
            idx
        } else {
            let idx = self.discrete_signatures.len();
            self.discrete_index.insert(discrete.to_vec(), idx);
            self.discrete_signatures.push(discrete.to_vec());
            idx
        };
        self.discrete_map.push(signature_idx);
        self.continuous_layouts.push(continuous.len());
        self.continuous_values.extend_from_slice(continuous);
        self.weights.push(weight);
    }

    pub(crate) fn finish(self) -> LatentBatchPayload {
        LatentBatchPayload::IndexedBatch {
            discrete_signatures: self.discrete_signatures,
            discrete_map: self.discrete_map,
            continuous_layouts: self.continuous_layouts,
            continuous_values: self.continuous_values,
            weights: self.weights,
        }
    }
}

impl LatentBatchPayload {
    pub fn from_batch(batch: &Batch) -> Self {
        let mut builder = IndexedBatchBuilder::new(batch.size());
        for point in batch.points() {
            let sampler_weight = point
                .factor_value("sampler_weight")
                .expect("batch point missing sampler_weight factor");
            builder.push(&point.discrete, &point.continuous, sampler_weight);
        }
        builder.finish()
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
            Self::HavanaInference { .. } | Self::HavanaInferenceIndexed { .. } => {
                Err(BatchError::layout(
                    "havana_inference latent payload must be materialized by a materializer",
                ))
            }
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
            Self::HavanaInference { .. } | Self::HavanaInferenceIndexed { .. } => {
                Err(BatchError::layout(
                    "havana_inference latent payload must be materialized by a materializer",
                ))
            }
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
    /// Copy a contiguous evaluator work unit from a generated batch. Only the
    /// selected coordinates and discrete signatures are copied.
    #[cfg(test)]
    pub(crate) fn slice(&self, start: usize, samples: usize) -> Result<Self, BatchError> {
        let coordinate_start = match &self.payload {
            LatentBatchPayload::IndexedBatch {
                continuous_layouts, ..
            } => continuous_layouts
                .get(..start)
                .ok_or_else(|| BatchError::layout("generated batch slice out of bounds"))?
                .iter()
                .sum(),
            _ => 0,
        };
        self.slice_at(start, samples, coordinate_start)
    }

    /// The generation cursor avoids rescanning earlier ragged coordinates.
    pub(crate) fn slice_at(
        &self,
        start: usize,
        samples: usize,
        coordinate_start: usize,
    ) -> Result<Self, BatchError> {
        let end = start
            .checked_add(samples)
            .filter(|end| *end <= self.nr_samples)
            .ok_or_else(|| BatchError::layout("generated batch slice out of bounds"))?;
        if samples == 0 {
            return Err(BatchError::layout("generated batch slice is empty"));
        }
        if let LatentBatchPayload::HavanaInferenceIndexed { rng_states, offset } = &self.payload {
            let first = (offset + start) / RNG_CHECKPOINT_STRIDE;
            let last = (offset + end - 1) / RNG_CHECKPOINT_STRIDE;
            return Ok(Self {
                nr_samples: samples,
                accumulator: self.accumulator.clone(),
                payload: LatentBatchPayload::HavanaInferenceIndexed {
                    rng_states: rng_states
                        .get(first..=last)
                        .ok_or_else(|| BatchError::layout("missing RNG checkpoint"))?
                        .to_vec(),
                    offset: (offset + start) % RNG_CHECKPOINT_STRIDE,
                },
            });
        }
        let LatentBatchPayload::IndexedBatch {
            discrete_signatures,
            discrete_map,
            continuous_layouts,
            continuous_values,
            weights,
        } = &self.payload
        else {
            return Err(BatchError::layout(
                "generated payload cannot be partitioned",
            ));
        };
        let mut signatures = Vec::new();
        let mut remap = HashMap::new();
        let mut maps = Vec::with_capacity(samples);
        for &index in &discrete_map[start..end] {
            let signature = discrete_signatures.get(index).ok_or_else(|| {
                BatchError::layout("generated batch references a missing discrete signature")
            })?;
            maps.push(*remap.entry(index).or_insert_with(|| {
                signatures.push(signature.clone());
                signatures.len() - 1
            }));
        }
        let layouts = continuous_layouts[start..end].to_vec();
        let coordinate_end = coordinate_start + layouts.iter().sum::<usize>();
        Ok(Self {
            nr_samples: samples,
            accumulator: self.accumulator.clone(),
            payload: LatentBatchPayload::IndexedBatch {
                discrete_signatures: signatures,
                discrete_map: maps,
                continuous_layouts: layouts,
                continuous_values: continuous_values[coordinate_start..coordinate_end].to_vec(),
                weights: weights[start..end].to_vec(),
            },
        })
    }
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
            LatentBatchPayload::HavanaInferenceIndexed { rng_states, offset } => {
                if *offset >= RNG_CHECKPOINT_STRIDE
                    || rng_states.len()
                        != self
                            .nr_samples
                            .checked_add(*offset)
                            .ok_or_else(|| BatchError::layout("RNG sample count overflow"))?
                            .div_ceil(RNG_CHECKPOINT_STRIDE)
                {
                    return Err(BatchError::layout(
                        "invalid RNG checkpoints or sample offset",
                    ));
                }
            }
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
        let mut bytes = Vec::new();
        self.write_bytes(&mut bytes)?;
        Ok(bytes)
    }

    /// Append the existing wire representation to a caller-owned buffer.
    pub(crate) fn write_bytes(&self, bytes: &mut Vec<u8>) -> Result<usize, BatchError> {
        let payload = match &self.payload {
            LatentBatchPayload::IndexedBatch {
                discrete_signatures,
                discrete_map,
                continuous_layouts,
                continuous_values,
                weights,
            } => LatentBatchPayloadBinary::IndexedBatch {
                discrete_signatures: Cow::Borrowed(discrete_signatures),
                discrete_map: Cow::Borrowed(discrete_map),
                continuous_layouts: Cow::Borrowed(continuous_layouts),
                continuous_values: Cow::Borrowed(continuous_values),
                weights: Cow::Borrowed(weights),
            },
            LatentBatchPayload::HavanaInference { rng_state } => {
                LatentBatchPayloadBinary::HavanaInference {
                    rng_state: Cow::Borrowed(rng_state),
                }
            }
            LatentBatchPayload::HavanaInferenceIndexed { rng_states, offset } => {
                LatentBatchPayloadBinary::HavanaInferenceIndexed {
                    rng_states: Cow::Borrowed(rng_states),
                    offset: *offset,
                }
            }
        };
        bincode::serde::encode_into_std_write(
            LatentBatchBinary {
                nr_samples: self.nr_samples,
                accumulator: Cow::Borrowed(&self.accumulator),
                payload,
            },
            bytes,
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
                discrete_signatures: discrete_signatures.into_owned(),
                discrete_map: discrete_map.into_owned(),
                continuous_layouts: continuous_layouts.into_owned(),
                continuous_values: continuous_values.into_owned(),
                weights: weights.into_owned(),
            },
            LatentBatchPayloadBinary::HavanaInference { rng_state } => {
                LatentBatchPayload::HavanaInference {
                    rng_state: rng_state.into_owned(),
                }
            }
            LatentBatchPayloadBinary::HavanaInferenceIndexed { rng_states, offset } => {
                LatentBatchPayload::HavanaInferenceIndexed {
                    rng_states: rng_states.into_owned(),
                    offset,
                }
            }
        };
        let restored = Self {
            nr_samples: latent.nr_samples,
            accumulator: latent.accumulator.into_owned(),
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
    fn bulk_chunks_preserve_ragged_points_weights_and_order() {
        let batch = Batch::from_points((0..11).map(|i| {
            Point::new(
                vec![i as f64; i % 4],
                vec![i as i64 % 3; i % 2],
                1.0 + i as f64,
            )
        }))
        .unwrap();
        let latent = LatentBatchSpec::from_batch(&batch).build();
        let chunks: Vec<_> = [(0, 1), (1, 4), (5, 4), (9, 2)]
            .into_iter()
            .map(|(start, size)| latent.slice(start, size).unwrap())
            .collect();
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.nr_samples)
                .collect::<Vec<_>>(),
            [1, 4, 4, 2]
        );
        let mut points = Vec::new();
        for chunk in chunks {
            let restored = LatentBatch::from_bytes(&chunk.to_bytes().unwrap()).unwrap();
            points.extend(
                restored
                    .payload
                    .into_batch()
                    .unwrap()
                    .points()
                    .iter()
                    .cloned(),
            );
        }
        assert_eq!(Batch::new(points).unwrap(), batch);
    }

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
    fn binary_encoding_preserves_existing_indexed_and_inference_payloads() {
        let batch = Batch::from_points([
            Point::new(vec![0.5], Vec::new(), 1.0),
            Point::new(vec![1.5], Vec::new(), 2.0),
        ])
        .unwrap();
        let indexed = LatentBatchSpec::from_batch(&batch).build();
        let inference = LatentBatch {
            nr_samples: 4096,
            accumulator: AccumulatorConfig::scalar(),
            payload: LatentBatchPayload::HavanaInference {
                rng_state: SerializableMonteCarloRng::new(42, 0),
            },
        };
        // Captured from the owned encoder before borrowing its arrays.
        let indexed_bytes: &[u8] = &[
            2, 1, 0, 0, 0, 0, 0, 1, 0, 2, 0, 0, 2, 1, 1, 2, 0, 0, 0, 0, 0, 0, 224, 63, 0, 0, 0, 0,
            0, 0, 248, 63, 2, 0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 64,
        ];
        let inference_bytes: &[u8] = &[
            251, 0, 16, 1, 0, 0, 0, 0, 1, 253, 149, 110, 235, 47, 38, 50, 215, 189, 253, 3, 241,
            102, 178, 51, 227, 239, 40, 253, 82, 159, 15, 19, 87, 103, 82, 71, 253, 148, 227, 74,
            14, 255, 225, 28, 88,
        ];
        for (batch, bytes) in [(indexed, indexed_bytes), (inference, inference_bytes)] {
            assert_eq!(batch.to_bytes().unwrap(), bytes);
            assert_eq!(LatentBatch::from_bytes(bytes).unwrap(), batch);
            let mut framed = vec![7, 8, 9];
            assert_eq!(batch.write_bytes(&mut framed).unwrap(), bytes.len());
            assert_eq!(&framed[..3], &[7, 8, 9]);
            assert_eq!(&framed[3..], bytes);
        }
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
