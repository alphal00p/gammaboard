use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaSamplerParams {
    pub seed: u64,
    pub bins: usize,
    pub samples_for_update: usize,
    pub initial_training_rate: f64,
    pub final_training_rate: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HavanaInferenceSamplerParams {
    pub seed: Option<u64>,
}

impl Default for HavanaSamplerParams {
    fn default() -> Self {
        Self {
            seed: 0,
            bins: 64,
            samples_for_update: 10_240,
            initial_training_rate: 0.1,
            final_training_rate: 0.1,
        }
    }
}
