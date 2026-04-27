use crate::core::RollingMetricSnapshot;
use serde::{Deserialize, Serialize};

/// Lightweight EWMA helper for non-negative timing/capacity metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RollingMetric {
    mean: Option<f64>,
    variance: f64,
    observations: u64,
    alpha: f64,
}

impl Default for RollingMetric {
    fn default() -> Self {
        Self {
            mean: None,
            variance: 0.0,
            observations: 0,
            alpha: 0.2,
        }
    }
}

impl RollingMetric {
    pub(crate) fn observe(&mut self, observation: f64) {
        self.observe_weighted(observation, 1.0);
    }

    pub(crate) fn observe_weighted(&mut self, observation: f64, weight: f64) {
        if !observation.is_finite() || observation < 0.0 {
            return;
        }
        if !weight.is_finite() || weight <= 0.0 {
            return;
        }
        let effective_alpha = 1.0 - (1.0 - self.alpha).powf(weight);
        let effective_alpha = effective_alpha.clamp(0.0, 1.0);
        match self.mean {
            Some(current_mean) => {
                let delta = observation - current_mean;
                let next_mean = current_mean + effective_alpha * delta;
                // EWMA-compatible variance update around the changing mean.
                let next_variance =
                    (1.0 - effective_alpha) * (self.variance + effective_alpha * delta * delta);
                self.mean = Some(next_mean);
                self.variance = next_variance.max(0.0);
            }
            None => {
                self.mean = Some(observation);
                self.variance = 0.0;
            }
        }
        self.observations += 1;
    }

    pub(crate) fn value(&self) -> Option<f64> {
        self.mean
    }

    pub(crate) fn std_dev(&self) -> f64 {
        self.variance.max(0.0).sqrt()
    }
}

impl From<&RollingMetric> for RollingMetricSnapshot {
    fn from(metric: &RollingMetric) -> Self {
        Self {
            count: metric.observations,
            mean: metric.value(),
            total: None,
            max: None,
            std_dev: metric.std_dev(),
        }
    }
}
