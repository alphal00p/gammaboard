//! Latent batch abstraction for sampler-owned queue payloads.

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::{Array2, array};

    #[test]
    fn latent_batch_roundtrips_batch_payload() {
        let batch =
            Batch::new(array![[0.5], [1.5]], Array2::zeros((2, 0)), None).expect("batch creation");
        let latent = LatentBatchSpec::from_batch(&batch).build();
        let json = latent.into_json();
        let restored = LatentBatch::from_json(&json).expect("latent batch");
        assert_eq!(restored.nr_samples, 2);
        let restored_batch = restored.payload.as_batch().expect("batch payload");
        assert_eq!(restored_batch.point_spec(), batch.point_spec());
        assert_eq!(restored_batch.weights(), batch.weights());
    }
}
