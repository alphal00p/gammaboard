//! Versions of persisted contracts, independent of the package release number.
use serde::{Deserialize, Deserializer};

pub const MODEL_VERSION: u32 = 1;
pub const PERFORMANCE_SCHEMA_VERSION: u32 = 2;
pub(crate) const QUEUE_MAGIC: &[u8; 4] = b"GBQ\x01";

pub(crate) fn deserialize_model_version<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version != MODEL_VERSION {
        return Err(serde::de::Error::custom(format!(
            "unsupported persisted model version {version}; expected {MODEL_VERSION}"
        )));
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_work_rejects_untagged_and_future_payloads() {
        use crate::{Batch, LatentBatch, Point};
        let work = LatentBatch::from_batch(&Batch::from_iter([Point::new(vec![0.5], vec![], 1.0)]))
            .for_evaluation(crate::AccumulatorConfig::scalar());
        let mut bytes = work.to_bytes().unwrap();
        assert_eq!(
            crate::core::EvaluationBatch::from_bytes(&bytes).unwrap(),
            work
        );
        assert!(crate::core::EvaluationBatch::from_bytes(&bytes[QUEUE_MAGIC.len()..]).is_err());
        bytes[3] = 99;
        assert!(crate::core::EvaluationBatch::from_bytes(&bytes).is_err());
    }

    #[test]
    fn persisted_version_is_required_and_checked() {
        #[derive(Deserialize)]
        struct Envelope {
            #[serde(deserialize_with = "deserialize_model_version")]
            format_version: u32,
        }
        assert_eq!(
            serde_json::from_str::<Envelope>("{\"format_version\":1}")
                .unwrap()
                .format_version,
            MODEL_VERSION
        );
        for value in ["{}", "{\"format_version\":0}", "{\"format_version\":99}"] {
            assert!(serde_json::from_str::<Envelope>(value).is_err());
        }
    }
}
