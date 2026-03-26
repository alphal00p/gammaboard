mod havana;
mod naive_monte_carlo;
mod raster;

use crate::Materializer;
use crate::core::{BuildError, SamplerAggregatorConfig};
use crate::evaluation::PointSpec;
use crate::sampling::materializer::{HavanaInferenceMaterializer, IdentityMaterializer};
use crate::sampling::{SamplerAggregator, SamplerAggregatorSnapshot, StageHandoff};

pub use self::havana::HavanaInferenceSamplerParams;
pub use self::havana::HavanaInferenceSource;
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
    pub fn requires_training(&self) -> bool {
        matches!(self, Self::HavanaTraining { .. })
    }

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
        handoff: Option<StageHandoff<'_>>,
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
                // Minimal in-place behavior: if no explicit sample_budget is provided,
                // default to a small positive budget (1) so initial-stage construction can proceed.
                // This preserves simplicity and avoids forcing callers to always supply a budget.
                let sample_budget = sample_budget.unwrap_or(1);
                Ok(Box::new(HavanaSampler::from_params_and_point_spec(
                    params.clone(),
                    &point_spec,
                    sample_budget,
                )?))
            }
            Self::HavanaInference { params } => {
                let Some(snapshot) = handoff.and_then(|handoff| handoff.sampler_snapshot.cloned())
                else {
                    return Err(BuildError::build(
                        "havana_inference sampler requires a persisted sampler snapshot handoff",
                    ));
                };
                Ok(Box::new(HavanaInferenceSampler::from_params_and_snapshot(
                    params.clone(),
                    snapshot,
                    &point_spec,
                )?))
            }
        }
    }
    pub fn build_materializer(
        &self,
        _point_spec: PointSpec,
        _sample_budget: Option<usize>,
        handoff: Option<StageHandoff<'_>>,
    ) -> Result<Box<dyn Materializer>, BuildError> {
        // If the provided handoff contains a Havana sampler snapshot (training or inference),
        // prefer the HavanaInferenceMaterializer so evaluators can materialize using the grid
        // persisted in that snapshot. This lets evaluator materializers be chosen based on
        // the stage handoff snapshot, not only the current sampler config.
        if let Some(snap_ref) = handoff.as_ref().and_then(|h| h.sampler_snapshot.as_ref()) {
            if matches!(
                snap_ref,
                SamplerAggregatorSnapshot::HavanaTraining { .. }
                    | SamplerAggregatorSnapshot::HavanaInference { .. }
            ) {
                return Ok(Box::new(HavanaInferenceMaterializer::new(handoff)?));
            }
        }

        Ok(match self {
            SamplerAggregatorConfig::HavanaInference { params: _ } => {
                Box::new(HavanaInferenceMaterializer::new(handoff)?)
            }
            _ => Box::new(IdentityMaterializer::new()),
        })
    }
}
