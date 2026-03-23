use crate::core::{BuildError, MaterializerConfig};
use crate::evaluation::Materializer;
use crate::sampling::StageHandoff;
use serde::{Deserialize, Serialize};

mod frozen_havana_inference;
mod identity;

use frozen_havana_inference::HavanaInferenceMaterializer;
pub use frozen_havana_inference::HavanaInferenceMaterializerParams;
use identity::IdentityMaterializer;
pub use identity::IdentityMaterializerParams;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaterializerSnapshot {
    Identity {},
    HavanaInference { grid: serde_json::Value },
}

impl MaterializerConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Identity { .. } => "identity",
            Self::HavanaInference { .. } => "havana_inference",
        }
    }

    pub fn build(
        &self,
        handoff: Option<StageHandoff<'_>>,
    ) -> Result<Box<dyn Materializer>, BuildError> {
        match self {
            Self::Identity { params } => {
                Ok(Box::new(IdentityMaterializer::from_params(params.clone())))
            }
            Self::HavanaInference { params } => Ok(Box::new(
                HavanaInferenceMaterializer::from_build_context(params.clone(), handoff)?,
            )),
        }
    }
}
