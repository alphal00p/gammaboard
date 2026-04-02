use super::{IngestComplex, Observable};
use num::complex::Complex64;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComplexObservableState {
    pub count: i64,
    pub real_sum: f64,
    pub imag_sum: f64,
    pub abs_sum: f64,
    pub abs_sq_sum: f64,
    pub real_sq_sum: f64,
    pub imag_sq_sum: f64,
    pub weight_sum: f64,
    #[serde(default)]
    pub nan_count: usize,
}

impl ComplexObservableState {
    pub fn add_sample(&mut self, value: Complex64, weight: f64) {
        let weight = weight.abs();
        let weighted_real = value.re * weight;
        let weighted_imag = value.im * weight;
        let weighted_abs = value.norm() * weight;
        let weighted_abs_sq = weighted_abs * weighted_abs;
        let weighted_real_sq = weighted_real * weighted_real;
        let weighted_imag_sq = weighted_imag * weighted_imag;
        if !weighted_real.is_finite()
            || !weighted_imag.is_finite()
            || !weighted_abs.is_finite()
            || !weighted_abs_sq.is_finite()
            || !weighted_real_sq.is_finite()
            || !weighted_imag_sq.is_finite()
            || !weight.is_finite()
        {
            self.nan_count += 1;
            return;
        }
        self.count += 1;
        self.real_sum += weighted_real;
        self.imag_sum += weighted_imag;
        self.abs_sum += weighted_abs;
        self.abs_sq_sum += weighted_abs_sq;
        self.real_sq_sum += weighted_real_sq;
        self.imag_sq_sum += weighted_imag_sq;
        self.weight_sum += weight;
    }

    pub fn real_mean(&self) -> f64 {
        mean_from_sums(self.real_sum, self.count)
    }

    pub fn imag_mean(&self) -> f64 {
        mean_from_sums(self.imag_sum, self.count)
    }

    pub fn abs_mean(&self) -> f64 {
        mean_from_sums(self.abs_sum, self.count)
    }

    pub fn real_stderr(&self) -> f64 {
        stderr_from_sums(self.real_sum, self.real_sq_sum, self.count)
    }

    pub fn imag_stderr(&self) -> f64 {
        stderr_from_sums(self.imag_sum, self.imag_sq_sum, self.count)
    }

    pub fn abs_stderr(&self) -> f64 {
        stderr_from_sums(self.abs_sum, self.abs_sq_sum, self.count)
    }

    pub fn abs_variance(&self) -> f64 {
        variance_from_sums(self.abs_sum, self.abs_sq_sum, self.count)
    }

    pub fn signal_to_noise(&self) -> f64 {
        signal_to_noise_ratio(self.abs_mean(), self.abs_stderr())
    }

    pub fn rsd(&self) -> f64 {
        relative_squared_dispersion(self.abs_variance(), self.abs_mean())
    }
}

impl IngestComplex for ComplexObservableState {
    fn ingest_complex(&mut self, value: Complex64, weight: f64) {
        self.add_sample(value, weight);
    }
}

impl Observable for ComplexObservableState {
    type Persistent = Self;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        self.count
    }

    fn merge(&mut self, other: Self) {
        self.count += other.count;
        self.real_sum += other.real_sum;
        self.imag_sum += other.imag_sum;
        self.abs_sum += other.abs_sum;
        self.abs_sq_sum += other.abs_sq_sum;
        self.real_sq_sum += other.real_sq_sum;
        self.imag_sq_sum += other.imag_sq_sum;
        self.weight_sum += other.weight_sum;
        self.nan_count += other.nan_count;
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

#[cfg(test)]
mod tests {
    use super::ComplexObservableState;
    use num::complex::Complex64;

    #[test]
    fn add_sample_uses_weighted_contribution_moments() {
        let mut observable = ComplexObservableState::default();

        observable.add_sample(Complex64::new(3.0, 4.0), 2.0);

        assert_eq!(observable.count, 1);
        assert_eq!(observable.real_sum, 6.0);
        assert_eq!(observable.imag_sum, 8.0);
        assert_eq!(observable.abs_sum, 10.0);
        assert_eq!(observable.real_sq_sum, 36.0);
        assert_eq!(observable.imag_sq_sum, 64.0);
        assert_eq!(observable.abs_sq_sum, 100.0);
        assert_eq!(observable.weight_sum, 2.0);
        assert_eq!(observable.nan_count, 0);
    }

    #[test]
    fn add_sample_normalizes_negative_weights() {
        let mut observable = ComplexObservableState::default();

        observable.add_sample(Complex64::new(1.5, -2.0), -3.0);

        assert_eq!(observable.real_sum, 4.5);
        assert_eq!(observable.imag_sum, -6.0);
        assert_eq!(observable.weight_sum, 3.0);
        assert_eq!(observable.nan_count, 0);
    }

    #[test]
    fn add_sample_skips_non_finite_weighted_contributions() {
        let mut observable = ComplexObservableState::default();

        observable.add_sample(Complex64::new(f64::NAN, 1.0), 1.0);
        observable.add_sample(Complex64::new(1.0, 0.0), f64::INFINITY);

        assert_eq!(observable.count, 0);
        assert_eq!(observable.real_sum, 0.0);
        assert_eq!(observable.imag_sum, 0.0);
        assert_eq!(observable.abs_sum, 0.0);
        assert_eq!(observable.real_sq_sum, 0.0);
        assert_eq!(observable.imag_sq_sum, 0.0);
        assert_eq!(observable.abs_sq_sum, 0.0);
        assert_eq!(observable.weight_sum, 0.0);
        assert_eq!(observable.nan_count, 2);
    }
}
