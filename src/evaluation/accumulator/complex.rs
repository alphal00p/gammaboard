use super::{Accumulator, IngestComplex};
use crate::core::DiscreteHistogramConfig;
use crate::evaluation::batch::Point;
use num::complex::Complex64;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::discrete_bins::{ComplexDiscreteBinStats, upsert_complex_discrete_bin};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComplexAccumulatorState {
    #[serde(default)]
    pub discrete_histograms: Option<DiscreteHistogramConfig>,
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
    #[serde(default)]
    pub max_real_weighted_positive: f64,
    #[serde(default)]
    pub max_real_weighted_negative: f64,
    #[serde(default)]
    pub max_imag_weighted_positive: f64,
    #[serde(default)]
    pub max_imag_weighted_negative: f64,
    #[serde(default)]
    pub max_real_weighted_positive_point: Option<Point>,
    #[serde(default)]
    pub max_real_weighted_negative_point: Option<Point>,
    #[serde(default)]
    pub max_imag_weighted_positive_point: Option<Point>,
    #[serde(default)]
    pub max_imag_weighted_negative_point: Option<Point>,
    #[serde(default)]
    pub discrete_bins: BTreeMap<String, ComplexDiscreteBinStats>,
    #[serde(default)]
    pub discrete_bins_overflow_count: usize,
}

impl ComplexAccumulatorState {
    pub fn from_config(discrete_histograms: Option<DiscreteHistogramConfig>) -> Self {
        Self {
            discrete_histograms,
            ..Self::default()
        }
    }

    pub fn add_sample(&mut self, value: Complex64, point: &Point) {
        let weight = point.total_weight().abs();
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
        self.update_real_extrema(weighted_real, point);
        self.update_imag_extrema(weighted_imag, point);
        if !point.discrete.is_empty() {
            upsert_complex_discrete_bin(
                &mut self.discrete_bins,
                &mut self.discrete_bins_overflow_count,
                &point.discrete,
                weighted_real,
                weighted_imag,
                weighted_abs,
            );
        }
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

    pub fn real_max_weight_impact(&self) -> f64 {
        component_max_weight_impact(
            self.real_mean().abs(),
            self.count,
            self.max_real_weighted_positive,
            self.max_real_weighted_negative,
        )
    }

    pub fn imag_max_weight_impact(&self) -> f64 {
        component_max_weight_impact(
            self.imag_mean().abs(),
            self.count,
            self.max_imag_weighted_positive,
            self.max_imag_weighted_negative,
        )
    }

    pub fn max_weight_impact(&self) -> f64 {
        self.real_max_weight_impact()
            .max(self.imag_max_weight_impact())
    }

    fn update_real_extrema(&mut self, weighted_real: f64, point: &Point) {
        if weighted_real >= 0.0
            && (self.max_real_weighted_positive_point.is_none()
                || weighted_real > self.max_real_weighted_positive)
        {
            self.max_real_weighted_positive = weighted_real;
            self.max_real_weighted_positive_point = Some(point.clone());
        }
        if weighted_real < 0.0
            && (self.max_real_weighted_negative_point.is_none()
                || weighted_real < self.max_real_weighted_negative)
        {
            self.max_real_weighted_negative = weighted_real;
            self.max_real_weighted_negative_point = Some(point.clone());
        }
    }

    fn update_imag_extrema(&mut self, weighted_imag: f64, point: &Point) {
        if weighted_imag >= 0.0
            && (self.max_imag_weighted_positive_point.is_none()
                || weighted_imag > self.max_imag_weighted_positive)
        {
            self.max_imag_weighted_positive = weighted_imag;
            self.max_imag_weighted_positive_point = Some(point.clone());
        }
        if weighted_imag < 0.0
            && (self.max_imag_weighted_negative_point.is_none()
                || weighted_imag < self.max_imag_weighted_negative)
        {
            self.max_imag_weighted_negative = weighted_imag;
            self.max_imag_weighted_negative_point = Some(point.clone());
        }
    }
}

impl IngestComplex for ComplexAccumulatorState {
    fn ingest_complex(&mut self, value: Complex64, point: &Point) {
        self.add_sample(value, point);
    }
}

impl Accumulator for ComplexAccumulatorState {
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
        merge_positive_extrema(
            &mut self.max_real_weighted_positive,
            &mut self.max_real_weighted_positive_point,
            other.max_real_weighted_positive,
            other.max_real_weighted_positive_point,
        );
        merge_negative_extrema(
            &mut self.max_real_weighted_negative,
            &mut self.max_real_weighted_negative_point,
            other.max_real_weighted_negative,
            other.max_real_weighted_negative_point,
        );
        merge_positive_extrema(
            &mut self.max_imag_weighted_positive,
            &mut self.max_imag_weighted_positive_point,
            other.max_imag_weighted_positive,
            other.max_imag_weighted_positive_point,
        );
        merge_negative_extrema(
            &mut self.max_imag_weighted_negative,
            &mut self.max_imag_weighted_negative_point,
            other.max_imag_weighted_negative,
            other.max_imag_weighted_negative_point,
        );
        for (key, candidate) in other.discrete_bins {
            self.discrete_bins
                .entry(key)
                .and_modify(|current| current.merge_in_place(candidate.clone()))
                .or_insert(candidate);
        }
        self.discrete_bins_overflow_count += other.discrete_bins_overflow_count;
        if self.discrete_histograms.is_none() {
            self.discrete_histograms = other.discrete_histograms;
        }
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

fn component_max_weight_impact(
    mean_abs: f64,
    count: i64,
    max_positive: f64,
    max_negative: f64,
) -> f64 {
    if count <= 0 || !mean_abs.is_finite() || mean_abs <= 0.0 {
        return 0.0;
    }
    let denom = mean_abs * count as f64;
    if !denom.is_finite() || denom <= 0.0 {
        return 0.0;
    }
    let numer = max_positive.abs().max(max_negative.abs());
    if !numer.is_finite() {
        return 0.0;
    }
    numer / denom
}

fn merge_positive_extrema(
    target_value: &mut f64,
    target_point: &mut Option<Point>,
    candidate_value: f64,
    candidate_point: Option<Point>,
) {
    let has_candidate = candidate_point.is_some() || candidate_value != 0.0;
    if !has_candidate {
        return;
    }
    if target_point.is_none() || candidate_value > *target_value {
        *target_value = candidate_value;
        *target_point = candidate_point;
    }
}

fn merge_negative_extrema(
    target_value: &mut f64,
    target_point: &mut Option<Point>,
    candidate_value: f64,
    candidate_point: Option<Point>,
) {
    let has_candidate = candidate_point.is_some() || candidate_value != 0.0;
    if !has_candidate {
        return;
    }
    if target_point.is_none() || candidate_value < *target_value {
        *target_value = candidate_value;
        *target_point = candidate_point;
    }
}

#[cfg(test)]
mod tests {
    use super::ComplexAccumulatorState;
    use crate::evaluation::Point;
    use num::complex::Complex64;

    #[test]
    fn add_sample_uses_weighted_contribution_moments() {
        let mut accumulator = ComplexAccumulatorState::default();

        let point = Point::new(vec![0.1], vec![7], 2.0);
        accumulator.add_sample(Complex64::new(3.0, 4.0), &point);

        assert_eq!(accumulator.count, 1);
        assert_eq!(accumulator.real_sum, 6.0);
        assert_eq!(accumulator.imag_sum, 8.0);
        assert_eq!(accumulator.abs_sum, 10.0);
        assert_eq!(accumulator.real_sq_sum, 36.0);
        assert_eq!(accumulator.imag_sq_sum, 64.0);
        assert_eq!(accumulator.abs_sq_sum, 100.0);
        assert_eq!(accumulator.weight_sum, 2.0);
        assert_eq!(accumulator.nan_count, 0);
    }

    #[test]
    fn add_sample_normalizes_negative_weights() {
        let mut accumulator = ComplexAccumulatorState::default();

        let point = Point::new(vec![0.1], vec![], -3.0);
        accumulator.add_sample(Complex64::new(1.5, -2.0), &point);

        assert_eq!(accumulator.real_sum, 4.5);
        assert_eq!(accumulator.imag_sum, -6.0);
        assert_eq!(accumulator.weight_sum, 3.0);
        assert_eq!(accumulator.nan_count, 0);
    }

    #[test]
    fn add_sample_skips_non_finite_weighted_contributions() {
        let mut accumulator = ComplexAccumulatorState::default();

        let finite_point = Point::new(vec![], vec![], 1.0);
        let inf_point = Point::new(vec![], vec![], f64::INFINITY);
        accumulator.add_sample(Complex64::new(f64::NAN, 1.0), &finite_point);
        accumulator.add_sample(Complex64::new(1.0, 0.0), &inf_point);

        assert_eq!(accumulator.count, 0);
        assert_eq!(accumulator.real_sum, 0.0);
        assert_eq!(accumulator.imag_sum, 0.0);
        assert_eq!(accumulator.abs_sum, 0.0);
        assert_eq!(accumulator.real_sq_sum, 0.0);
        assert_eq!(accumulator.imag_sq_sum, 0.0);
        assert_eq!(accumulator.abs_sq_sum, 0.0);
        assert_eq!(accumulator.weight_sum, 0.0);
        assert_eq!(accumulator.nan_count, 2);
    }

    #[test]
    fn add_sample_tracks_real_imag_max_weight_points() {
        let mut accumulator = ComplexAccumulatorState::default();
        let point_a = Point::new(vec![0.25], vec![1], 1.0);
        let point_b = Point::new(vec![0.75], vec![2], 2.0);

        accumulator.add_sample(Complex64::new(2.0, -1.0), &point_a);
        accumulator.add_sample(Complex64::new(1.0, 3.0), &point_b);

        assert_eq!(accumulator.max_real_weighted_positive, 2.0);
        assert_eq!(accumulator.max_real_weighted_positive_point, Some(point_a));
        assert_eq!(accumulator.max_imag_weighted_positive, 6.0);
        assert_eq!(accumulator.max_imag_weighted_positive_point, Some(point_b));
        assert_eq!(accumulator.max_imag_weighted_negative, -1.0);
    }
}
