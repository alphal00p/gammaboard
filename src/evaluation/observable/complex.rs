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
}

impl ComplexObservableState {
    pub fn add_sample(&mut self, value: Complex64, weight: f64) {
        let weight = weight.abs();
        let weighted_real = value.re * weight;
        let weighted_imag = value.im * weight;
        let weighted_abs = value.norm() * weight;
        self.count += 1;
        self.real_sum += weighted_real;
        self.imag_sum += weighted_imag;
        self.abs_sum += weighted_abs;
        self.abs_sq_sum += weighted_abs * weighted_abs;
        self.real_sq_sum += weighted_real * weighted_real;
        self.imag_sq_sum += weighted_imag * weighted_imag;
        self.weight_sum += weight;
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

    fn merge(&mut self, other: Self) {
        self.count += other.count;
        self.real_sum += other.real_sum;
        self.imag_sum += other.imag_sum;
        self.abs_sum += other.abs_sum;
        self.abs_sq_sum += other.abs_sq_sum;
        self.real_sq_sum += other.real_sq_sum;
        self.imag_sq_sum += other.imag_sq_sum;
        self.weight_sum += other.weight_sum;
    }

    fn get_persistent(&self) -> Self::Persistent {
        self.clone()
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
    }

    #[test]
    fn add_sample_normalizes_negative_weights() {
        let mut observable = ComplexObservableState::default();

        observable.add_sample(Complex64::new(1.5, -2.0), -3.0);

        assert_eq!(observable.real_sum, 4.5);
        assert_eq!(observable.imag_sum, -6.0);
        assert_eq!(observable.weight_sum, 3.0);
    }
}
