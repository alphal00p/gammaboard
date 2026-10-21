#[cfg(feature = "gammaloop")]
use gammaloop_api::state::ProcessRef;
#[cfg(feature = "gammaloop")]
use gammalooprs::integrands::process::GaussianReferenceFunction;
use serde::{Deserialize, Serialize};
#[cfg(not(feature = "gammaloop"))]
use serde_json::Value as ProcessRef;
#[cfg(not(feature = "gammaloop"))]
use serde_json::Value as GaussianReferenceFunction;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrainingProjection {
    #[default]
    Real,
    Imag,
    Abs,
    AbsSq,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GammaLoopParams {
    pub state_folder: PathBuf,
    pub process_id: Option<ProcessRef>,
    pub integrand_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_groups: Option<Vec<usize>>,
    pub momentum_space: bool,
    pub use_f128: bool,
    pub training_projection: TrainingProjection,
    /// Replace physics with known normalization and second-moment targets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_gaussian: Option<GaussianReferenceFunction>,
    pub preprocessing: GammaLoopPreprocessing,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct GammaLoopPreprocessing {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    pub read_only: bool,
}

impl Default for GammaLoopPreprocessing {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            read_only: true,
        }
    }
}

impl Default for GammaLoopParams {
    fn default() -> Self {
        Self {
            state_folder: PathBuf::from("./gammaloop_state"),
            process_id: None,
            integrand_name: None,
            graph_groups: None,
            momentum_space: false,
            use_f128: false,
            training_projection: TrainingProjection::default(),
            reference_gaussian: None,
            preprocessing: GammaLoopPreprocessing::default(),
        }
    }
}
