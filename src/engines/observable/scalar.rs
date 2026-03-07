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

    pub fn merge(&mut self, other: Self) {
        self.count += other.count;
        self.sum_weighted_value += other.sum_weighted_value;
        self.sum_abs += other.sum_abs;
        self.sum_sq += other.sum_sq;
    }
}
