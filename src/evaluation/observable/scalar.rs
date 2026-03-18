use super::{IngestScalar, Observable};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScalarObservableState {
    pub count: i64,
    pub sum_weighted_value: f64,
    pub sum_abs: f64,
    pub sum_sq: f64,
}

impl ScalarObservableState {
    pub fn add_sample(&mut self, value: f64, weight: f64) {
        let weight = weight.abs();
        let weighted_value = value * weight;
        self.count += 1;
        self.sum_weighted_value += weighted_value;
        self.sum_abs += weighted_value.abs();
        self.sum_sq += weighted_value * weighted_value;
    }

    pub fn mean(&self) -> f64 {
        mean_from_sums(self.sum_weighted_value, self.count)
    }

    pub fn mean_abs(&self) -> f64 {
        mean_from_sums(self.sum_abs, self.count)
    }

    pub fn variance(&self) -> f64 {
        variance_from_sums(self.sum_weighted_value, self.sum_sq, self.count)
    }

    pub fn stderr(&self) -> f64 {
        stderr_from_sums(self.sum_weighted_value, self.sum_sq, self.count)
    }

    pub fn signal_to_noise(&self) -> f64 {
        signal_to_noise_ratio(self.mean_abs(), self.stderr())
    }

    pub fn rsd(&self) -> f64 {
        relative_squared_dispersion(self.variance(), self.mean_abs())
    }
}

impl IngestScalar for ScalarObservableState {
    fn ingest_scalar(&mut self, value: f64, weight: f64) {
        self.add_sample(value, weight);
    }
}

impl Observable for ScalarObservableState {
    type Persistent = Self;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        self.count
    }

    fn merge(&mut self, other: Self) {
        self.count += other.count;
        self.sum_weighted_value += other.sum_weighted_value;
        self.sum_abs += other.sum_abs;
        self.sum_sq += other.sum_sq;
    }

    fn get_persistent(&self) -> Self::Persistent {
        self.clone()
    }
}

fn mean_from_sums(sum: f64, count: i64) -> f64 {
    if count <= 0 { 0.0 } else { sum / count as f64 }
}

fn variance_from_sums(sum: f64, sum_sq: f64, count: i64) -> f64 {
    if count <= 0 {
        return 0.0;
    }
    let count_f = count as f64;
    let mean = sum / count_f;
    let second_moment = sum_sq / count_f;
    (second_moment - mean * mean).max(0.0)
}

fn stderr_from_sums(sum: f64, sum_sq: f64, count: i64) -> f64 {
    if count <= 0 {
        0.0
    } else {
        (variance_from_sums(sum, sum_sq, count) / count as f64).sqrt()
    }
}

fn signal_to_noise_ratio(mean_abs: f64, abs_err: f64) -> f64 {
    if abs_err <= 0.0 {
        0.0
    } else {
        (mean_abs * mean_abs) / (abs_err * abs_err)
    }
}

fn relative_squared_dispersion(variance: f64, mean_abs: f64) -> f64 {
    if mean_abs == 0.0 {
        0.0
    } else {
        variance / (mean_abs * mean_abs)
    }
}
