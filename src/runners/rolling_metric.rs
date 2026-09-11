/// Lightweight EWMA helper for non-negative timing/capacity metrics.
#[derive(Debug, Clone, Default)]
pub(crate) struct RollingMetric {
    mean: Option<f64>,
    variance: f64,
}

impl RollingMetric {
    /// Smooth milliseconds per sample with alpha=0.2 per 1,000 samples.
    /// Applying this conversion here keeps queue and evaluator timings consistent.
    pub(crate) fn observe_batch(&mut self, total_ms: f64, samples: usize) {
        if samples == 0 {
            return;
        }
        let observation = total_ms / samples as f64;
        if !observation.is_finite() || observation < 0.0 {
            return;
        }
        let effective_alpha = 1.0 - 0.8_f64.powf(samples as f64 / 1000.0);
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
    }

    pub(crate) fn value(&self) -> Option<f64> {
        self.mean
    }

    pub(crate) fn std_dev(&self) -> f64 {
        self.variance.max(0.0).sqrt()
    }
}
