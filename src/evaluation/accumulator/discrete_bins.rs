use crate::core::DEFAULT_DISCRETE_HISTOGRAM_MAX_TOTAL_BINS;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
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
