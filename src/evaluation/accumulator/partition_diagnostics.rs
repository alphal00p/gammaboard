use crate::core::{EngineError, StratificationConfig};
use serde::{Deserialize, Serialize};

/// Counts only: partitioned training still uses the ordinary pooled estimator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartitionDiagnostics {
    pub partition: StratificationConfig,
    pub attempts: Vec<u64>,
    pub matches: Vec<u64>,
    pub feedback_samples: u64,
}

impl PartitionDiagnostics {
    pub fn validate(&self, samples: i64) -> Result<(), EngineError> {
        self.partition.validate()?;
        let n = self.partition.centers().len();
        let total = self
            .attempts
            .iter()
            .try_fold(0u64, |a, &b| a.checked_add(b));
        if self.attempts.len() != n
            || self.matches.len() != n
            || self.matches.iter().zip(&self.attempts).any(|(m, n)| m > n)
            || total != u64::try_from(samples).ok()
            || self.feedback_samples > total.unwrap_or(0)
        {
            return Err(EngineError::eval("invalid partition diagnostic counts"));
        }
        Ok(())
    }

    pub fn merge(&mut self, other: Self) -> Result<(), EngineError> {
        if self.partition != other.partition {
            return Err(EngineError::eval(
                "partition diagnostics cannot merge different partitions",
            ));
        }
        for (a, b) in self.attempts.iter_mut().zip(other.attempts) {
            *a = a
                .checked_add(b)
                .ok_or_else(|| EngineError::eval("partition count overflow"))?;
        }
        for (a, b) in self.matches.iter_mut().zip(other.matches) {
            *a = a
                .checked_add(b)
                .ok_or_else(|| EngineError::eval("partition count overflow"))?;
        }
        self.feedback_samples = self
            .feedback_samples
            .checked_add(other.feedback_samples)
            .ok_or_else(|| EngineError::eval("feedback count overflow"))?;
        Ok(())
    }
}
