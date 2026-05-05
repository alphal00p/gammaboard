use crate::core::DEFAULT_DISCRETE_HISTOGRAM_MAX_TOTAL_BINS;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScalarDiscreteBinStats {
    pub discrete: Vec<i64>,
    pub count: i64,
    pub sum_weighted_value: f64,
    pub sum_sq: f64,
}

impl ScalarDiscreteBinStats {
    pub fn add_weighted_sample(&mut self, weighted_value: f64) {
        self.count += 1;
        self.sum_weighted_value += weighted_value;
        self.sum_sq += weighted_value * weighted_value;
    }

    pub fn merge_in_place(&mut self, other: Self) {
        self.count += other.count;
        self.sum_weighted_value += other.sum_weighted_value;
        self.sum_sq += other.sum_sq;
    }

    pub fn mean(&self) -> f64 {
        if self.count <= 0 {
            0.0
        } else {
            self.sum_weighted_value / self.count as f64
        }
    }

    pub fn stderr(&self) -> f64 {
        if self.count <= 0 {
            return 0.0;
        }
        let count_f = self.count as f64;
        let mean = self.sum_weighted_value / count_f;
        let variance = (self.sum_sq / count_f - mean * mean).max(0.0);
        (variance / count_f).sqrt()
    }

    pub fn contribution_mean(&self, total_count: i64) -> f64 {
        if total_count <= 0 {
            0.0
        } else {
            self.sum_weighted_value / total_count as f64
        }
    }

    pub fn contribution_stderr(&self, total_count: i64) -> f64 {
        if total_count <= 0 {
            return 0.0;
        }
        let total_count_f = total_count as f64;
        let mean = self.sum_weighted_value / total_count_f;
        let variance = (self.sum_sq / total_count_f - mean * mean).max(0.0);
        (variance / total_count_f).sqrt()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComplexDiscreteBinStats {
    pub discrete: Vec<i64>,
    pub count: i64,
    pub real_sum: f64,
    pub imag_sum: f64,
    pub abs_sum: f64,
    pub real_sq_sum: f64,
    pub imag_sq_sum: f64,
    pub abs_sq_sum: f64,
}

impl ComplexDiscreteBinStats {
    pub fn add_weighted_sample(
        &mut self,
        weighted_real: f64,
        weighted_imag: f64,
        weighted_abs: f64,
    ) {
        self.count += 1;
        self.real_sum += weighted_real;
        self.imag_sum += weighted_imag;
        self.abs_sum += weighted_abs;
        self.real_sq_sum += weighted_real * weighted_real;
        self.imag_sq_sum += weighted_imag * weighted_imag;
        self.abs_sq_sum += weighted_abs * weighted_abs;
    }

    pub fn merge_in_place(&mut self, other: Self) {
        self.count += other.count;
        self.real_sum += other.real_sum;
        self.imag_sum += other.imag_sum;
        self.abs_sum += other.abs_sum;
        self.real_sq_sum += other.real_sq_sum;
        self.imag_sq_sum += other.imag_sq_sum;
        self.abs_sq_sum += other.abs_sq_sum;
    }

    pub fn projected_mean(&self, projection: ComplexDiscreteProjection) -> f64 {
        if self.count <= 0 {
            return 0.0;
        }
        match projection {
            ComplexDiscreteProjection::Real => self.real_sum / self.count as f64,
            ComplexDiscreteProjection::Imag => self.imag_sum / self.count as f64,
            ComplexDiscreteProjection::Abs => self.abs_sum / self.count as f64,
        }
    }

    pub fn projected_stderr(&self, projection: ComplexDiscreteProjection) -> f64 {
        if self.count <= 0 {
            return 0.0;
        }
        let count_f = self.count as f64;
        let (sum, sum_sq) = match projection {
            ComplexDiscreteProjection::Real => (self.real_sum, self.real_sq_sum),
            ComplexDiscreteProjection::Imag => (self.imag_sum, self.imag_sq_sum),
            ComplexDiscreteProjection::Abs => (self.abs_sum, self.abs_sq_sum),
        };
        let mean = sum / count_f;
        let variance = (sum_sq / count_f - mean * mean).max(0.0);
        (variance / count_f).sqrt()
    }

    pub fn projected_contribution_mean(
        &self,
        projection: ComplexDiscreteProjection,
        total_count: i64,
    ) -> f64 {
        if total_count <= 0 {
            return 0.0;
        }
        let sum = match projection {
            ComplexDiscreteProjection::Real => self.real_sum,
            ComplexDiscreteProjection::Imag => self.imag_sum,
            ComplexDiscreteProjection::Abs => self.abs_sum,
        };
        sum / total_count as f64
    }

    pub fn projected_contribution_stderr(
        &self,
        projection: ComplexDiscreteProjection,
        total_count: i64,
    ) -> f64 {
        if total_count <= 0 {
            return 0.0;
        }
        let total_count_f = total_count as f64;
        let (sum, sum_sq) = match projection {
            ComplexDiscreteProjection::Real => (self.real_sum, self.real_sq_sum),
            ComplexDiscreteProjection::Imag => (self.imag_sum, self.imag_sq_sum),
            ComplexDiscreteProjection::Abs => (self.abs_sum, self.abs_sq_sum),
        };
        let mean = sum / total_count_f;
        let variance = (sum_sq / total_count_f - mean * mean).max(0.0);
        (variance / total_count_f).sqrt()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ComplexDiscreteProjection {
    Real,
    Imag,
    #[default]
    Abs,
}

pub fn discrete_bin_key(discrete: &[i64]) -> String {
    discrete
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn upsert_scalar_discrete_bin(
    bins: &mut BTreeMap<String, ScalarDiscreteBinStats>,
    overflow_count: &mut usize,
    discrete: &[i64],
    weighted_value: f64,
) {
    let key = discrete_bin_key(discrete);
    if !bins.contains_key(&key) && bins.len() >= DEFAULT_DISCRETE_HISTOGRAM_MAX_TOTAL_BINS {
        *overflow_count += 1;
        return;
    }
    let entry = bins.entry(key).or_insert_with(|| ScalarDiscreteBinStats {
        discrete: discrete.to_vec(),
        ..ScalarDiscreteBinStats::default()
    });
    entry.add_weighted_sample(weighted_value);
}

pub fn upsert_complex_discrete_bin(
    bins: &mut BTreeMap<String, ComplexDiscreteBinStats>,
    overflow_count: &mut usize,
    discrete: &[i64],
    weighted_real: f64,
    weighted_imag: f64,
    weighted_abs: f64,
) {
    let key = discrete_bin_key(discrete);
    if !bins.contains_key(&key) && bins.len() >= DEFAULT_DISCRETE_HISTOGRAM_MAX_TOTAL_BINS {
        *overflow_count += 1;
        return;
    }
    let entry = bins.entry(key).or_insert_with(|| ComplexDiscreteBinStats {
        discrete: discrete.to_vec(),
        ..ComplexDiscreteBinStats::default()
    });
    entry.add_weighted_sample(weighted_real, weighted_imag, weighted_abs);
}
