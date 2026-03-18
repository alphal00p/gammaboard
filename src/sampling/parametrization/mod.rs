use crate::core::{BuildError, ParametrizationConfig};
use crate::evaluation::Parametrization;
use crate::sampling::SamplerAggregatorSnapshot;
use serde::{Deserialize, Serialize};

mod frozen_havana_inference;
mod identity;
mod spherical;
mod unit_ball;

use frozen_havana_inference::HavanaInferenceParametrization;
pub use frozen_havana_inference::HavanaInferenceParametrizationParams;
use identity::IdentityParametrization;
pub use identity::IdentityParametrizationParams;
use spherical::SphericalParametrization;
pub use spherical::SphericalParametrizationParams;
use unit_ball::UnitBallParametrization;
pub use unit_ball::UnitBallParametrizationParams;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParametrizationSnapshot {
    Identity {},
    UnitBall {},
    Spherical {},
    HavanaInference {
        grid: serde_json::Value,
        inner: Box<ParametrizationSnapshot>,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ParametrizationBuildContext<'a> {
    pub sampler_aggregator_snapshot: Option<&'a SamplerAggregatorSnapshot>,
    pub parametrization_snapshot: Option<&'a ParametrizationSnapshot>,
}

impl ParametrizationConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Identity { .. } => "identity",
            Self::UnitBall { .. } => "unit_ball",
            Self::Spherical { .. } => "spherical",
            Self::HavanaInference { .. } => "havana_inference",
        }
    }

    pub fn build(
        &self,
        ctx: ParametrizationBuildContext<'_>,
    ) -> Result<Box<dyn Parametrization>, BuildError> {
        match self {
            Self::Identity { params } => Ok(Box::new(IdentityParametrization::from_params(
                params.clone(),
            ))),
            Self::UnitBall { params } => Ok(Box::new(UnitBallParametrization::from_params(
                params.clone(),
            ))),
            Self::Spherical { params } => Ok(Box::new(SphericalParametrization::from_params(
                params.clone(),
            ))),
            Self::HavanaInference { params } => Ok(Box::new(
                HavanaInferenceParametrization::from_build_context(params.clone(), ctx)?,
            )),
        }
    }
}
