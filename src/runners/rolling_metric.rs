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
        if !observation.is_finite() || observation < 0.0 {
            return;
        }
        match self.mean {
            Some(current_mean) => {
                let delta = observation - current_mean;
                let next_mean = current_mean + self.alpha * delta;
                // EWMA-compatible variance update around the changing mean.
                let next_variance =
                    (1.0 - self.alpha) * (self.variance + self.alpha * delta * delta);
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
