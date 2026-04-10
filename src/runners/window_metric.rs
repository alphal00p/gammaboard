use crate::core::RollingMetricSnapshot;
use serde::{Deserialize, Serialize};

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
    pub(crate) fn observe(&mut self, observation: f64) {
        if !observation.is_finite() || observation < 0.0 {
            return;
        }
        self.count += 1;
        self.sum += observation;
        let delta = observation - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = observation - self.mean;
        self.m2 += delta * delta2;
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
