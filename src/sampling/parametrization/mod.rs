use crate::engines::{BuildError, Parametrization};

mod identity;
mod spherical;
mod unit_ball;

use identity::IdentityParametrization;
pub use identity::IdentityParametrizationParams;
use spherical::SphericalParametrization;
pub use spherical::SphericalParametrizationParams;
use unit_ball::UnitBallParametrization;
pub use unit_ball::UnitBallParametrizationParams;

impl crate::engines::ParametrizationConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Identity { .. } => "identity",
            Self::UnitBall { .. } => "unit_ball",
            Self::Spherical { .. } => "spherical",
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
        }
    }
}
