use crate::core::RollingMetricSnapshot;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Snapshot-window metric accumulator for non-negative timing/capacity metrics.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct WindowMetric {
    count: u64,
    sum: f64,
    mean: f64,
    m2: f64,
    max: Option<f64>,
}

impl WindowMetric {
    pub(crate) fn observe_duration(&mut self, duration: Duration) {
        self.observe(duration.as_secs_f64() * 1000.0);
    }

    pub(crate) fn observe(&mut self, observation: f64) {
        self.observe_weighted(observation, 1);
    }

    /// Record a batch-normalized cost with its sample/batch count as weight.
    pub(crate) fn observe_weighted(&mut self, observation: f64, weight: usize) {
        if !observation.is_finite() || observation < 0.0 || weight == 0 {
            return;
        }
        self.count += weight as u64;
        self.sum += observation * weight as f64;
        let delta = observation - self.mean;
        self.mean += delta * weight as f64 / self.count as f64;
        let delta2 = observation - self.mean;
        self.m2 += delta * delta2 * weight as f64;
        self.max = Some(
            self.max
                .map_or(observation, |current| current.max(observation)),
        );
    }

    pub(crate) fn snapshot(&self) -> RollingMetricSnapshot {
        if self.count == 0 {
            return RollingMetricSnapshot::default();
        }
        let variance = if self.count > 1 {
            self.m2 / self.count as f64
        } else {
            0.0
        };
        RollingMetricSnapshot {
            count: self.count,
            mean: Some(self.mean),
            total: Some(self.sum),
            max: self.max,
            std_dev: variance.max(0.0).sqrt(),
        }
    }

    pub(crate) fn snapshot_and_reset(&mut self) -> RollingMetricSnapshot {
        let snapshot = self.snapshot();
        *self = Self::default();
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unequal_batches_preserve_total_time_and_sample_weight() {
        let mut metric = WindowMetric::default();
        metric.observe_weighted(10.0, 1);
        metric.observe_weighted(0.1, 1000);
        let snapshot = metric.snapshot_and_reset();
        assert_eq!(snapshot.count, 1001);
        assert_eq!(snapshot.total, Some(110.0));
        assert!((snapshot.mean.unwrap() - 110.0 / 1001.0).abs() < 1e-12);
        assert_eq!(metric.snapshot().count, 0);
    }
}
