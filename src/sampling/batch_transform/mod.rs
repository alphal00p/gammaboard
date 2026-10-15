use crate::core::{BatchTransformConfig, EngineError};
use crate::evaluation::BatchTransform;

mod process;
mod spherical;
mod unit_ball;

use process::ProcessBatchTransform;
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

    pub fn build(
        &self,
        domain: &crate::utils::domain::Domain,
    ) -> Result<Box<dyn BatchTransform>, EngineError> {
        let transform: Box<dyn BatchTransform> = match self {
            Self::UnitBall { params } => {
                Box::new(UnitBallBatchTransform::from_params(params.clone()))
            }
            Self::Spherical { params } => {
                Box::new(SphericalBatchTransform::from_params(params.clone()))
            }
            Self::ProcessBatchTransform { params } => {
                Box::new(ProcessBatchTransform::from_params(params.clone(), domain)?)
            }
        };
        transform.validate_domain(domain)?;
        Ok(transform)
    }
}
