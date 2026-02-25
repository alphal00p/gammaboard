#[derive(Debug, Clone, Default)]
pub(crate) struct SampleTimeStats {
    batches: i64,
    samples: i64,
    sum: f64,
    sum_sq: f64,
}

impl SampleTimeStats {
    pub(crate) fn observe(&mut self, sample_count: usize, elapsed_ms: f64) {
        if sample_count == 0 || !elapsed_ms.is_finite() || elapsed_ms < 0.0 {
            return;
        }
        let sample_time_ms = elapsed_ms / sample_count as f64;
        self.batches += 1;
        self.samples += sample_count as i64;
        self.sum += sample_time_ms;
        self.sum_sq += sample_time_ms * sample_time_ms;
    }

    pub(crate) fn mean(&self) -> f64 {
        if self.batches <= 0 {
            0.0
        } else {
            self.sum / self.batches as f64
        }
    }

    pub(crate) fn std(&self) -> f64 {
        if self.batches <= 1 {
            return 0.0;
        }
        let n = self.batches as f64;
        let numerator = self.sum_sq - (self.sum * self.sum) / n;
        (numerator / (n - 1.0)).max(0.0).sqrt()
    }

    pub(crate) fn has_data(&self) -> bool {
        self.batches > 0
    }

    pub(crate) fn batches(&self) -> i64 {
        self.batches
    }

    pub(crate) fn samples(&self) -> i64 {
        self.samples
    }
}
