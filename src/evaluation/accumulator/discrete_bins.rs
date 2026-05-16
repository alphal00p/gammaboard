use crate::core::DEFAULT_DISCRETE_HISTOGRAM_MAX_TOTAL_BINS;
use crate::evaluation::accumulator::ScalarAccumulatorState;
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DiscreteProjectionBinState {
    pub discrete: Vec<i64>,
    pub state: ScalarAccumulatorState,
}

impl DiscreteProjectionBinState {
    pub fn merge_in_place(&mut self, other: Self) {
        self.state.merge_plain(other.state);
    }

    pub fn mean(&self) -> f64 {
        self.state.mean()
    }

    pub fn stderr(&self) -> f64 {
        self.state.stderr()
    }

    pub fn contribution_mean(&self, total_count: i64) -> f64 {
        if total_count <= 0 {
            0.0
        } else {
            self.state.sum_weighted_value / total_count as f64
        }
    }

    pub fn contribution_stderr(&self, total_count: i64) -> f64 {
        if total_count <= 0 {
            return 0.0;
        }
        let total_count_f = total_count as f64;
        let mean = self.state.sum_weighted_value / total_count_f;
        let variance = (self.state.sum_sq / total_count_f - mean * mean).max(0.0);
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
    bins: &mut BTreeMap<String, DiscreteProjectionBinState>,
    overflow_count: &mut usize,
    discrete: &[i64],
    value: f64,
    point: &Point,
) {
    let key = discrete_bin_key(discrete);
    if !bins.contains_key(&key) && bins.len() >= DEFAULT_DISCRETE_HISTOGRAM_MAX_TOTAL_BINS {
        *overflow_count += 1;
        return;
    }
    let entry = bins
        .entry(key)
        .or_insert_with(|| DiscreteProjectionBinState {
            discrete: discrete.to_vec(),
            state: ScalarAccumulatorState::plain(),
        });
    entry
        .state
        .add_sample_without_discrete_projection(value, point);
}
