use super::{Accumulator, IngestScalar};
use crate::core::DiscreteHistogramConfig;
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::discrete_bins::{ScalarDiscreteBinStats, upsert_scalar_discrete_bin};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScalarAccumulatorState {
    #[serde(default)]
    pub discrete_histograms: Option<DiscreteHistogramConfig>,
    pub count: i64,
    pub sum_weighted_value: f64,
    pub sum_abs: f64,
    pub sum_sq: f64,
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
    pub discrete_bins: BTreeMap<String, ScalarDiscreteBinStats>,
    #[serde(default)]
    pub discrete_bins_overflow_count: usize,
}

impl ScalarAccumulatorState {
    pub fn from_config(discrete_histograms: Option<DiscreteHistogramConfig>) -> Self {
        Self {
            discrete_histograms,
            ..Self::default()
        }
    }

    pub fn add_sample(&mut self, value: f64, point: &Point) {
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
        self.update_extrema(weighted_value, point);
        if !point.discrete.is_empty() {
            upsert_scalar_discrete_bin(
                &mut self.discrete_bins,
                &mut self.discrete_bins_overflow_count,
                &point.discrete,
                weighted_value,
            );
        }
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
        self.count += other.count;
        self.sum_weighted_value += other.sum_weighted_value;
        self.sum_abs += other.sum_abs;
        self.sum_sq += other.sum_sq;
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
