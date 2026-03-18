mod havana;
mod naive_monte_carlo;
mod raster;

use crate::core::{BuildError, SamplerAggregatorConfig};
use crate::evaluation::PointSpec;
use crate::sampling::{SamplerAggregator, SamplerAggregatorSnapshot};

pub use self::havana::HavanaInferenceSamplerParams;
pub use self::havana::HavanaSamplerParams;
use self::havana::{
    HavanaInferenceSampler, HavanaInferenceSamplerSnapshot, HavanaSampler, HavanaSamplerSnapshot,
};
use self::naive_monte_carlo::NaiveMonteCarloSamplerAggregator;
pub use self::naive_monte_carlo::NaiveMonteCarloSamplerParams;
use self::raster::{
    RasterLineSampler, RasterLineSamplerSnapshot, RasterPlaneSampler, RasterPlaneSamplerSnapshot,
};
pub use self::raster::{RasterLineSamplerParams, RasterPlaneSamplerParams};

impl SamplerAggregatorSnapshot {
    pub fn into_runtime(
        self,
        point_spec: &PointSpec,
    ) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { raw } => {
                let snapshot: NaiveMonteCarloSamplerAggregator = serde_json::from_value(raw)
                    .map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode naive_monte_carlo sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(NaiveMonteCarloSamplerAggregator::from_snapshot(
                    snapshot, point_spec,
                )?))
            }
            Self::RasterPlane { raw } => {
                let snapshot: RasterPlaneSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode raster plane sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(RasterPlaneSampler::from_snapshot(
                    snapshot, point_spec,
                )?))
            }
            Self::RasterLine { raw } => {
                let snapshot: RasterLineSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode raster line sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(RasterLineSampler::from_snapshot(
                    snapshot, point_spec,
                )?))
            }
            Self::HavanaTraining { raw } => {
                let snapshot: HavanaSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(HavanaSampler::from_snapshot(
                    snapshot, point_spec,
                )?))
            }
            Self::HavanaInference { raw } => {
                let snapshot: HavanaInferenceSamplerSnapshot = serde_json::from_value(raw)
                    .map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana inference sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(HavanaInferenceSampler::from_snapshot(
                    snapshot, point_spec,
                )?))
            }
        }
    }
}

impl SamplerAggregatorConfig {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::NaiveMonteCarlo { .. } => "naive_monte_carlo",
            Self::RasterPlane { .. } => "raster_plane",
            Self::RasterLine { .. } => "raster_line",
            Self::HavanaTraining { .. } => "havana_training",
            Self::HavanaInference { .. } => "havana_inference",
        }
    }

    pub fn build(
        &self,
        point_spec: PointSpec,
        sample_budget: Option<usize>,
    ) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { params } => Ok(Box::new(
                NaiveMonteCarloSamplerAggregator::from_params_and_point_spec(
                    params.clone(),
                    &point_spec,
                )?,
            )),
            Self::RasterPlane { params } => Ok(Box::new(
                RasterPlaneSampler::from_params_and_point_spec(params.clone(), &point_spec)?,
            )),
            Self::RasterLine { params } => Ok(Box::new(
                RasterLineSampler::from_params_and_point_spec(params.clone(), &point_spec)?,
            )),
            Self::HavanaTraining { params } => {
                let sample_budget = sample_budget.ok_or_else(|| {
                    BuildError::build(
                        "havana_training sampler requires sample task nr_samples for training budget",
                    )
                })?;
                Ok(Box::new(HavanaSampler::from_params_and_point_spec(
                    params.clone(),
                    &point_spec,
                    sample_budget,
                )?))
            }
            Self::HavanaInference { .. } => Err(BuildError::build(
                "havana_inference sampler requires a persisted snapshot handoff",
            )),
        }
    }

    pub fn build_from_params_and_snapshot(
        &self,
        point_spec: PointSpec,
        sample_budget: Option<usize>,
        snapshot: SamplerAggregatorSnapshot,
    ) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { .. }
            | Self::RasterPlane { .. }
            | Self::RasterLine { .. }
            | Self::HavanaTraining { .. } => {
                let _ = snapshot;
                self.build(point_spec, sample_budget)
            }
            Self::HavanaInference { params } => {
                Ok(Box::new(HavanaInferenceSampler::from_params_and_snapshot(
                    params.clone(),
                    snapshot,
                    &point_spec,
                )?))
            }
        }
    }
}
