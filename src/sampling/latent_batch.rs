//! Latent batch abstraction for sampler-owned queue payloads.

use bincode::config::{Configuration, standard};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::core::ObservableConfig;
use crate::evaluation::{Batch, BatchError};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentBatch {
    pub nr_samples: usize,
    pub observable: ObservableConfig,
    pub payload: LatentBatchPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentBatchSpec {
    pub nr_samples: usize,
    pub observable: ObservableConfig,
    pub payload: LatentBatchPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LatentBatchPayload {
    Batch { batch: JsonValue },
    HavanaInference { seed: u64 },
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
    observable: ObservableConfig,
    payload: LatentBatchPayloadBinary,
}

#[derive(Debug, Serialize, Deserialize)]
enum LatentBatchPayloadBinary {
    Batch { batch: Batch },
    HavanaInference { seed: u64 },
}

impl LatentBatchPayload {
    pub fn from_batch(batch: &Batch) -> Self {
        Self::Batch {
            batch: batch.to_json(),
        }
    }

    pub fn into_batch(self) -> Result<Batch, BatchError> {
        match self {
            Self::Batch { batch } => Batch::from_json(&batch),
            Self::HavanaInference { .. } => Err(BatchError::layout(
                "havana_inference latent payload must be materialized by a materializer",
            )),
        }
    }

    pub fn as_batch(&self) -> Result<Batch, BatchError> {
        match self {
            Self::Batch { batch } => Batch::from_json(batch),
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
            observable: ObservableConfig::Scalar,
            payload: LatentBatchPayload::from_batch(batch),
        }
    }

    pub fn build(self) -> LatentBatch {
        LatentBatch {
            nr_samples: self.nr_samples,
            observable: self.observable,
            payload: self.payload,
        }
    }

    pub fn with_observable_config(mut self, observable: ObservableConfig) -> Self {
        self.observable = observable;
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
            LatentBatchPayload::Batch { batch } => LatentBatchPayloadBinary::Batch {
                batch: Batch::from_json(batch)?,
            },
            LatentBatchPayload::HavanaInference { seed } => {
                LatentBatchPayloadBinary::HavanaInference { seed: *seed }
            }
        };
        bincode::serde::encode_to_vec(
            LatentBatchBinary {
                nr_samples: self.nr_samples,
                observable: self.observable.clone(),
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
            LatentBatchPayloadBinary::Batch { batch } => LatentBatchPayload::from_batch(&batch),
            LatentBatchPayloadBinary::HavanaInference { seed } => {
                LatentBatchPayload::HavanaInference { seed }
            }
        };
        let restored = Self {
            nr_samples: latent.nr_samples,
            observable: latent.observable,
            payload,
        };
        restored.validate_nr_samples()?;
        Ok(restored)
    }
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
}
