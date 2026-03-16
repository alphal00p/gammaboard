use crate::core::{BuildError, ParametrizationConfig};
use crate::evaluation::Parametrization;

mod frozen_havana_inference;
mod identity;
mod spherical;
mod unit_ball;

use frozen_havana_inference::FrozenHavanaInferenceParametrization;
pub use frozen_havana_inference::{
    FrozenHavanaInferenceParametrizationParams, HavanaInferenceParametrizationParams,
};
use identity::IdentityParametrization;
pub use identity::IdentityParametrizationParams;
use spherical::SphericalParametrization;
pub use spherical::SphericalParametrizationParams;
use unit_ball::UnitBallParametrization;
pub use unit_ball::UnitBallParametrizationParams;

impl ParametrizationConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Identity { .. } => "identity",
            Self::UnitBall { .. } => "unit_ball",
            Self::Spherical { .. } => "spherical",
            Self::HavanaInference { .. } => "havana_inference",
            Self::FrozenHavanaInference { .. } => "frozen_havana_inference",
        }
    }

    pub fn build(&self) -> Result<Box<dyn Parametrization>, BuildError> {
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
            Self::FrozenHavanaInference { params } => Ok(Box::new(
                FrozenHavanaInferenceParametrization::from_params(params.clone())?,
            )),
            Self::HavanaInference { .. } => Err(BuildError::build(
                "havana_inference parametrization must be resolved from a persisted snapshot before building",
            )),
        }
    }
}
