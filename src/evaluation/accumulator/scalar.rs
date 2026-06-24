use super::{Accumulator, IngestScalar};
use crate::core::{AccumulatorMomentConfig, DiscreteProjectionConfig};
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::discrete_bins::{DiscreteProjectionBinState, upsert_scalar_discrete_bin};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ScalarAccumulatorState {
    #[serde(default)]
    pub discrete_projections: Option<DiscreteProjectionConfig>,
    #[serde(default)]
    pub moments: AccumulatorMomentConfig,
    pub count: i64,
    pub sum_weighted_value: f64,
    pub sum_abs: f64,
    pub sum_sq: f64,
    #[serde(default)]
    pub sum_cu: f64,
    #[serde(default)]
    pub sum_four: f64,
    #[serde(default)]
    pub nan_count: usize,
    #[serde(default)]
    pub max_weighted_positive: f64,
    #[serde(default)]
    pub max_weighted_negative: f64,
    #[serde(default)]
    pub max_weighted_positive_point: Option<Point>,
    #[serde(default)]
    pub max_weighted_negative_point: Option<Point>,
    #[serde(default)]
    pub discrete_bins: BTreeMap<String, DiscreteProjectionBinState>,
    #[serde(default)]
    pub discrete_bins_overflow_count: usize,
}

impl ScalarAccumulatorState {
    pub fn from_config(
        discrete_projections: Option<DiscreteProjectionConfig>,
        moments: AccumulatorMomentConfig,
    ) -> Self {
        Self {
            discrete_projections,
            moments,
            ..Self::default()
        }
    }

    pub fn plain() -> Self {
        Self::default()
    }

    pub fn add_sample(&mut self, value: f64, point: &Point) {
        self.add_sample_without_discrete_projection(value, point);
        if !point.discrete.is_empty() {
            upsert_scalar_discrete_bin(
                &mut self.discrete_bins,
                &mut self.discrete_bins_overflow_count,
                &point.discrete,
                value,
                point,
            );
        }
    }

    pub fn add_sample_without_discrete_projection(&mut self, value: f64, point: &Point) {
        let weight = point.total_weight().abs();
        let weighted_value = value * weight;
        let weighted_sq = weighted_value * weighted_value;
        if !weighted_value.is_finite() || !weighted_sq.is_finite() {
            self.nan_count += 1;
            return;
        }
        self.count += 1;
        self.sum_weighted_value += weighted_value;
        self.sum_abs += weighted_value.abs();
        self.sum_sq += weighted_sq;
        if self.moments.max_order() >= 4 {
            self.sum_cu += weighted_sq * weighted_value;
            self.sum_four += weighted_sq * weighted_sq;
        }
        self.update_extrema(weighted_value, point);
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

    pub fn variance_stderr(&self) -> Option<f64> {
        variance_stderr_from_sums(
            self.sum_weighted_value,
            self.sum_sq,
            self.sum_cu,
            self.sum_four,
            self.count,
        )
    }

    pub fn signal_to_noise(&self) -> f64 {
        signal_to_noise_ratio(self.mean_abs(), self.stderr())
    }

    pub fn rsd(&self) -> f64 {
        reduced_square_deviation(self.variance(), self.mean_abs())
    }

    pub fn rsd_stderr(&self) -> Option<f64> {
        reduced_square_deviation_stderr(
            self.variance(),
            self.variance_stderr(),
            self.mean_abs(),
            self.stderr(),
        )
    }

    pub fn max_weight_impact(&self) -> f64 {
        if self.count <= 0 {
            return 0.0;
        }
        let denom = self.mean().abs() * self.count as f64;
        if !denom.is_finite() || denom <= 0.0 {
            return 0.0;
        }
        let numer = self
            .max_weighted_positive
            .abs()
            .max(self.max_weighted_negative.abs());
        if !numer.is_finite() {
            return 0.0;
        }
        numer / denom
    }

    fn update_extrema(&mut self, weighted_value: f64, point: &Point) {
        if weighted_value >= 0.0
            && (self.max_weighted_positive_point.is_none()
                || weighted_value > self.max_weighted_positive)
        {
            self.max_weighted_positive = weighted_value;
            self.max_weighted_positive_point = Some(point.clone());
        }
        if weighted_value < 0.0
            && (self.max_weighted_negative_point.is_none()
                || weighted_value < self.max_weighted_negative)
        {
            self.max_weighted_negative = weighted_value;
            self.max_weighted_negative_point = Some(point.clone());
        }
    }

    pub fn merge_plain(&mut self, other: Self) {
        self.count += other.count;
        self.sum_weighted_value += other.sum_weighted_value;
        self.sum_abs += other.sum_abs;
        self.sum_sq += other.sum_sq;
        self.sum_cu += other.sum_cu;
        self.sum_four += other.sum_four;
        if other.moments.max_order() > self.moments.max_order() {
            self.moments = other.moments;
        }
        self.nan_count += other.nan_count;
        merge_positive_extrema(
            &mut self.max_weighted_positive,
            &mut self.max_weighted_positive_point,
            other.max_weighted_positive,
            other.max_weighted_positive_point,
        );
        merge_negative_extrema(
            &mut self.max_weighted_negative,
            &mut self.max_weighted_negative_point,
            other.max_weighted_negative,
            other.max_weighted_negative_point,
        );
    }
}

impl IngestScalar for ScalarAccumulatorState {
    fn ingest_scalar(&mut self, value: f64, point: &Point) {
        self.add_sample(value, point);
    }
}

impl Accumulator for ScalarAccumulatorState {
    type Persistent = Self;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        self.count
    }

    fn merge(&mut self, other: Self) {
        let other_discrete_bins = other.discrete_bins;
        let other_discrete_bins_overflow_count = other.discrete_bins_overflow_count;
        let other_discrete_projections = other.discrete_projections.clone();
        self.merge_plain(Self {
            discrete_bins: BTreeMap::new(),
            discrete_bins_overflow_count: 0,
            ..other
        });
        for (key, candidate) in other_discrete_bins {
            self.discrete_bins
                .entry(key)
                .and_modify(|current| current.merge_in_place(candidate.clone()))
                .or_insert(candidate);
        }
        self.discrete_bins_overflow_count += other_discrete_bins_overflow_count;
        if self.discrete_projections.is_none() {
            self.discrete_projections = other_discrete_projections;
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

fn variance_stderr_from_sums(
    sum: f64,
    sum_sq: f64,
    sum_cu: f64,
    sum_four: f64,
    count: i64,
) -> Option<f64> {
    if count <= 1 || sum_four == 0.0 {
        return None;
    }
    let count_f = count as f64;
    let mean = sum / count_f;
    let second = sum_sq / count_f;
    let third = sum_cu / count_f;
    let fourth = sum_four / count_f;
    let variance = (second - mean * mean).max(0.0);
    let centered_fourth =
        fourth - 4.0 * mean * third + 6.0 * mean * mean * second - 3.0 * mean.powi(4);
    let variance_of_variance = (centered_fourth - variance * variance).max(0.0) / count_f;
    let stderr = variance_of_variance.sqrt();
    stderr.is_finite().then_some(stderr)
}

fn signal_to_noise_ratio(mean_abs: f64, abs_err: f64) -> f64 {
    if abs_err <= 0.0 {
        0.0
    } else {
        (mean_abs * mean_abs) / (abs_err * abs_err)
    }
}

fn reduced_square_deviation(variance: f64, mean_abs: f64) -> f64 {
    if mean_abs == 0.0 {
        0.0
    } else {
        variance.max(0.0).sqrt() / mean_abs
    }
}

/// First-order uncertainty on `RSD = sqrt(variance) / mean_abs` via error
/// propagation. With `RSD = sqrt(V)/M`, the relative uncertainty is
/// `sqrt((dV / 2V)^2 + (dM / M)^2)`; the `1/2` comes from `d sqrt(V)/dV`. The
/// V-M covariance is neglected, as is conventional.
fn reduced_square_deviation_stderr(
    variance: f64,
    variance_stderr: Option<f64>,
    mean_abs: f64,
    mean_stderr: f64,
) -> Option<f64> {
    let variance_stderr = variance_stderr?;
    if !(variance > 0.0) || !(mean_abs > 0.0) {
        return None;
    }
    let rsd = variance.sqrt() / mean_abs;
    let relative_variance = variance_stderr / (2.0 * variance);
    let relative_mean = mean_stderr / mean_abs;
    let stderr =
        rsd * (relative_variance * relative_variance + relative_mean * relative_mean).sqrt();
    stderr.is_finite().then_some(stderr)
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
    use super::ScalarAccumulatorState;
    use crate::evaluation::Point;

    #[test]
    fn add_sample_accepts_finite_weighted_contributions() {
        let mut accumulator = ScalarAccumulatorState::default();

        let point = Point::new(vec![0.5], vec![1], -3.0);
        accumulator.add_sample(2.0, &point);

        assert_eq!(accumulator.count, 1);
        assert_eq!(accumulator.sum_weighted_value, 6.0);
        assert_eq!(accumulator.sum_abs, 6.0);
        assert_eq!(accumulator.sum_sq, 36.0);
        assert_eq!(accumulator.nan_count, 0);
    }

    #[test]
    fn add_sample_skips_non_finite_weighted_contributions() {
        let mut accumulator = ScalarAccumulatorState::default();

        let point = Point::new(vec![], vec![], 1.0);
        let inf_point = Point::new(vec![], vec![], f64::INFINITY);
        accumulator.add_sample(f64::NAN, &point);
        accumulator.add_sample(1.0, &inf_point);

        assert_eq!(accumulator.count, 0);
        assert_eq!(accumulator.sum_weighted_value, 0.0);
        assert_eq!(accumulator.sum_abs, 0.0);
        assert_eq!(accumulator.sum_sq, 0.0);
        assert_eq!(accumulator.nan_count, 2);
    }

    #[test]
    fn add_sample_tracks_max_weight_points() {
        let mut accumulator = ScalarAccumulatorState::default();
        let point_a = Point::new(vec![0.1], vec![0], 1.0);
        let point_b = Point::new(vec![0.9], vec![1], 2.0);

        accumulator.add_sample(3.0, &point_a);
        accumulator.add_sample(-4.0, &point_b);

        assert_eq!(accumulator.max_weighted_positive, 3.0);
        assert_eq!(accumulator.max_weighted_positive_point, Some(point_a));
        assert_eq!(accumulator.max_weighted_negative, -8.0);
        assert_eq!(accumulator.max_weighted_negative_point, Some(point_b));
    }
}
