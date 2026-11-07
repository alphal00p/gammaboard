use crate::core::{EngineError, StratificationConfig};
use serde::{Deserialize, Serialize};

/// Diagnostics grouped by requested stratum; training still uses the pooled estimator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartitionDiagnostics {
    pub partition: StratificationConfig,
    pub attempts: Vec<u64>,
    pub matches: Vec<u64>,
    pub feedback_samples: u64,
    #[serde(default)]
    pub training_moments: Option<Vec<TrainingMoments>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrainingMoments {
    pub sum_abs: f64,
    pub sum_sq: f64,
}

impl TrainingMoments {
    pub fn add(&mut self, value: f64) -> Result<(), EngineError> {
        self.sum_abs += value.abs();
        self.sum_sq += value * value;
        if !self.sum_abs.is_finite() || !self.sum_sq.is_finite() {
            return Err(EngineError::eval("training moment overflow"));
        }
        Ok(())
    }
    fn ess(&self, count: u64) -> Option<f64> {
        if count == 0 {
            return None;
        }
        if self.sum_sq == 0.0 {
            return Some(0.0);
        }
        Some(
            ((self.sum_abs / self.sum_sq.sqrt()) / (count as f64).sqrt())
                .powi(2)
                .clamp(0.0, 1.0),
        )
    }
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
            || self.training_moments.as_ref().is_some_and(|moments| {
                moments.len() != n
                    || moments.iter().any(|m| {
                        !m.sum_abs.is_finite()
                            || !m.sum_sq.is_finite()
                            || m.sum_abs < 0.0
                            || m.sum_sq < 0.0
                    })
            })
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
        let was_empty = self.attempts.iter().all(|n| *n == 0);
        let incoming_empty = other.attempts.iter().all(|n| *n == 0);
        self.training_moments = if was_empty {
            other.training_moments.clone()
        } else if incoming_empty {
            self.training_moments.clone()
        } else {
            self.training_moments
                .as_ref()
                .zip(other.training_moments.as_ref())
                .map(|(left, right)| {
                    left.iter()
                        .zip(right)
                        .map(|(a, b)| TrainingMoments {
                            sum_abs: a.sum_abs + b.sum_abs,
                            sum_sq: a.sum_sq + b.sum_sq,
                        })
                        .collect()
                })
        };
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

    pub fn training_ess_by_stratum(&self) -> Vec<Option<f64>> {
        self.attempts
            .iter()
            .enumerate()
            .map(|(k, &n)| self.training_moments.as_ref()?.get(k)?.ess(n))
            .collect()
    }

    pub fn training_ess(&self) -> Option<f64> {
        let count = self.attempts.iter().sum::<u64>();
        if count == 0 {
            return None;
        }
        let mut ess = 0.0;
        for (&n, value) in self.attempts.iter().zip(self.training_ess_by_stratum()) {
            if n > 0 {
                ess += n as f64 / count as f64 * value?;
            }
        }
        Some(ess.clamp(0.0, 1.0))
    }
}
