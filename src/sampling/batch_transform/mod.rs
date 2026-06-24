use crate::core::{BatchTransformConfig, BuildError};
use crate::evaluation::BatchTransform;

mod process;
mod spherical;
mod unit_ball;

use process::ProcessBatchTransform;
pub use process::ProcessBatchTransformParams;
use spherical::SphericalBatchTransform;
pub use spherical::SphericalBatchTransformParams;
use unit_ball::UnitBallBatchTransform;
pub use unit_ball::UnitBallBatchTransformParams;

impl BatchTransformConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::UnitBall { .. } => "unit_ball",
            Self::Spherical { .. } => "spherical",
            Self::ProcessBatchTransform { .. } => "process_batch_transform",
        }
    }

    pub fn build(&self) -> Result<Box<dyn BatchTransform>, BuildError> {
        match self {
            Self::UnitBall { params } => Ok(Box::new(UnitBallBatchTransform::from_params(
                params.clone(),
            ))),
            Self::Spherical { params } => Ok(Box::new(SphericalBatchTransform::from_params(
                params.clone(),
            ))),
            Self::ProcessBatchTransform { params } => Ok(Box::new(
                ProcessBatchTransform::from_params(params.clone())?,
            )),
        }
    }
}
