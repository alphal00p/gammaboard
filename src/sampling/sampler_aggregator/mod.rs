mod havana;
mod naive_monte_carlo;
mod raster;

use crate::Materializer;
use crate::core::{BuildError, SamplerAggregatorConfig};
use crate::sampling::materializer::{HavanaInferenceMaterializer, IdentityMaterializer};
use crate::sampling::{SamplerAggregator, SamplerAggregatorSnapshot, StageHandoff};
use crate::utils::domain::Domain;

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
    pub fn into_runtime(self, domain: &Domain) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { raw } => {
                let snapshot: NaiveMonteCarloSamplerAggregator = serde_json::from_value(raw)
                    .map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode naive_monte_carlo sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(NaiveMonteCarloSamplerAggregator::from_snapshot(
                    snapshot, domain,
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
                    snapshot, domain,
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
                    snapshot, domain,
                )?))
            }
            Self::HavanaTraining { raw } => {
                let snapshot: HavanaSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(HavanaSampler::from_snapshot(snapshot, domain)?))
            }
            Self::HavanaInference { raw } => {
                let snapshot: HavanaInferenceSamplerSnapshot = serde_json::from_value(raw)
                    .map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode havana inference sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(HavanaInferenceSampler::from_snapshot(
                    snapshot, domain,
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
        domain: Domain,
        sample_budget: Option<usize>,
        handoff: Option<StageHandoff<'_>>,
    ) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        match self {
            Self::NaiveMonteCarlo { params } => Ok(Box::new(
                NaiveMonteCarloSamplerAggregator::from_params_and_domain(params.clone(), &domain)?,
            )),
            Self::RasterPlane { params } => Ok(Box::new(
                RasterPlaneSampler::from_params_and_domain(params.clone(), &domain)?,
            )),
            Self::RasterLine { params } => Ok(Box::new(RasterLineSampler::from_params_and_domain(
                params.clone(),
                &domain,
            )?)),
            Self::HavanaTraining { params } => {
                // Minimal in-place behavior: if no explicit sample_budget is provided,
                // default to a small positive budget (1) so initial-stage construction can proceed.
                // This preserves simplicity and avoids forcing callers to always supply a budget.
                let sample_budget = sample_budget.unwrap_or(1);
                Ok(Box::new(HavanaSampler::from_params_and_domain(
                    params.clone(),
                    &domain,
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
                    &domain,
                )?))
            }
        }
    }
    pub fn build_materializer(
        &self,
        handoff: Option<StageHandoff<'_>>,
    ) -> Result<Box<dyn Materializer>, BuildError> {
        Ok(match self {
            SamplerAggregatorConfig::NaiveMonteCarlo { params } => Box::new(
                IdentityMaterializer::new_with_failure(params.fail_on_materialize_batch_nr),
            ),
            SamplerAggregatorConfig::HavanaInference { params: _ } => {
                Box::new(HavanaInferenceMaterializer::new(handoff)?)
            }
            _ => Box::new(IdentityMaterializer::new()),
        })
    }
}
