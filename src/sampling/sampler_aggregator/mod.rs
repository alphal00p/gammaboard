mod havana;
mod naive_monte_carlo;
mod process;
mod raster;

use crate::Materializer;
use crate::core::{BuildError, MaterializerConfig, SamplerAggregatorConfig};
use crate::evaluation::AccumulatorState;
use crate::sampling::materializer::{
    HavanaInferenceMaterializer, IdentityMaterializer, ProcessMaterializer,
};
use crate::sampling::{SamplerAggregator, SamplerAggregatorSnapshot, StageHandoff};
use crate::utils::domain::Domain;
use serde_json::Value as JsonValue;

pub use self::havana::HavanaInferenceSamplerParams;
pub use self::havana::HavanaInferenceSource;
pub use self::havana::HavanaSamplerParams;
use self::havana::{
    HavanaInferenceSampler, HavanaInferenceSamplerSnapshot, HavanaSampler, HavanaSamplerSnapshot,
};
use self::naive_monte_carlo::NaiveMonteCarloSamplerAggregator;
pub use self::naive_monte_carlo::NaiveMonteCarloSamplerParams;
pub use self::process::ProcessSamplerParams;
use self::process::{ProcessSampler, ProcessSamplerSnapshot};
pub use self::raster::{
    PdfAdaptationImagePersistedOutput, PdfAdaptationRasterLineSamplerParams,
    PdfAdaptationRasterPlaneSamplerParams, RasterLineSamplerParams, RasterPlaneSamplerParams,
};
use self::raster::{
    PdfAdaptationRasterLineSampler, PdfAdaptationRasterLineSamplerSnapshot,
    PdfAdaptationRasterPlaneSampler, PdfAdaptationRasterPlaneSamplerSnapshot, RasterLineSampler,
    RasterLineSamplerSnapshot, RasterPlaneSampler, RasterPlaneSamplerSnapshot,
};

fn global_abs_integrand_norm_from_handoff(handoff: Option<StageHandoff<'_>>) -> Option<f64> {
    let observable = handoff.and_then(|value| value.observable_state)?;
    match observable {
        AccumulatorState::Scalar(state) => {
            let value = state.mean_abs();
            (value.is_finite() && value > 0.0).then_some(value)
        }
        AccumulatorState::Vector(state) => {
            let value = state.projection.state.mean_abs();
            (value.is_finite() && value > 0.0).then_some(value)
        }
        _ => None,
    }
}

impl SamplerAggregatorSnapshot {
    pub fn into_runtime(
        self,
        domain: &Domain,
        evaluator_metadata: JsonValue,
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
            Self::PdfAdaptationRasterPlane { raw } => {
                let snapshot: PdfAdaptationRasterPlaneSamplerSnapshot = serde_json::from_value(raw)
                    .map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode pdf adaptation raster plane sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(PdfAdaptationRasterPlaneSampler::from_snapshot(
                    snapshot, domain,
                )?))
            }
            Self::PdfAdaptationRasterLine { raw } => {
                let snapshot: PdfAdaptationRasterLineSamplerSnapshot = serde_json::from_value(raw)
                    .map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode pdf adaptation raster line sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(PdfAdaptationRasterLineSampler::from_snapshot(
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
            Self::ProcessSampler { raw } => {
                let snapshot: ProcessSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        BuildError::build(format!(
                            "failed to decode process_sampler sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(ProcessSampler::from_snapshot(
                    snapshot,
                    domain,
                    evaluator_metadata,
                )?))
            }
        }
    }
}

impl SamplerAggregatorConfig {
    pub fn requires_training(&self) -> bool {
        match self {
            Self::HavanaTraining { .. }
            | Self::PdfAdaptationRasterPlane { .. }
            | Self::PdfAdaptationRasterLine { .. } => true,
            Self::ProcessSampler { params, .. } => params.requires_training_values,
            _ => false,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::NaiveMonteCarlo { .. } => "naive_monte_carlo",
            Self::RasterPlane { .. } => "raster_plane",
            Self::RasterLine { .. } => "raster_line",
            Self::PdfAdaptationRasterPlane { .. } => "pdf_adaptation_raster_plane",
            Self::PdfAdaptationRasterLine { .. } => "pdf_adaptation_raster_line",
            Self::HavanaTraining { .. } => "havana_training",
            Self::HavanaInference { .. } => "havana_inference",
            Self::ProcessSampler { .. } => "process_sampler",
        }
    }

    pub fn build(
        &self,
        domain: Domain,
        sample_budget: Option<usize>,
        handoff: Option<StageHandoff<'_>>,
        evaluator_metadata: JsonValue,
    ) -> Result<Box<dyn SamplerAggregator>, BuildError> {
        if let Some(snapshot) = handoff
            .and_then(|handoff| handoff.sampler_snapshot.cloned())
            .filter(|snapshot| snapshot.matches_config(self))
        {
            return snapshot.into_runtime(&domain, evaluator_metadata);
        }

        match self {
            Self::NaiveMonteCarlo { params, .. } => Ok(Box::new(
                NaiveMonteCarloSamplerAggregator::from_params_and_domain(params.clone(), &domain)?,
            )),
            Self::RasterPlane { params, .. } => Ok(Box::new(
                RasterPlaneSampler::from_params_and_domain(params.clone(), &domain)?,
            )),
            Self::RasterLine { params, .. } => Ok(Box::new(
                RasterLineSampler::from_params_and_domain(params.clone(), &domain)?,
            )),
            Self::PdfAdaptationRasterPlane { params, .. } => {
                let Some(snapshot) = handoff.and_then(|handoff| handoff.sampler_snapshot.cloned())
                else {
                    return Err(BuildError::build(
                        "pdf_adaptation_raster_plane sampler requires a persisted sampler snapshot handoff",
                    ));
                };
                let global_abs_integrand_norm = global_abs_integrand_norm_from_handoff(handoff);
                Ok(Box::new(
                    PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
                        crate::sampling::PdfAdaptationRasterPlaneSamplerParams {
                            geometry: params.geometry.clone(),
                        },
                        snapshot,
                        global_abs_integrand_norm,
                        &domain,
                    )?,
                ))
            }
            Self::PdfAdaptationRasterLine { params, .. } => {
                let Some(snapshot) = handoff.and_then(|handoff| handoff.sampler_snapshot.cloned())
                else {
                    return Err(BuildError::build(
                        "pdf_adaptation_raster_line sampler requires a persisted sampler snapshot handoff",
                    ));
                };
                let global_abs_integrand_norm = global_abs_integrand_norm_from_handoff(handoff);
                Ok(Box::new(
                    PdfAdaptationRasterLineSampler::from_params_and_snapshot(
                        crate::sampling::PdfAdaptationRasterLineSamplerParams {
                            geometry: params.geometry.clone(),
                        },
                        snapshot,
                        global_abs_integrand_norm,
                        &domain,
                    )?,
                ))
            }
            Self::HavanaTraining { params, .. } => {
                let sample_budget = sample_budget.ok_or_else(|| {
                    BuildError::build("havana_training sampler requires a sample budget")
                })?;
                Ok(Box::new(HavanaSampler::from_params_and_domain(
                    params.clone(),
                    &domain,
                    sample_budget,
                )?))
            }
            Self::HavanaInference { params, .. } => {
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
            Self::ProcessSampler { params, .. } => {
                Ok(Box::new(ProcessSampler::from_params_and_domain(
                    params.clone(),
                    &domain,
                    evaluator_metadata,
                )?))
            }
        }
    }
    pub fn build_materializer(
        &self,
        domain: &Domain,
        handoff: Option<StageHandoff<'_>>,
    ) -> Result<Box<dyn Materializer>, BuildError> {
        if let Some(materializer) = self.materializer_config() {
            return match materializer {
                MaterializerConfig::ProcessMaterializer { params } => Ok(Box::new(
                    ProcessMaterializer::from_params_and_domain(params.clone(), domain)?,
                )),
            };
        }
        Ok(match self {
            SamplerAggregatorConfig::NaiveMonteCarlo { params, .. } => Box::new(
                IdentityMaterializer::new_with_failure(params.fail_on_materialize_batch_nr),
            ),
            SamplerAggregatorConfig::HavanaInference { params: _, .. } => {
                Box::new(HavanaInferenceMaterializer::new(handoff)?)
            }
            _ => Box::new(IdentityMaterializer::new()),
        })
    }

    fn materializer_config(&self) -> Option<&MaterializerConfig> {
        match self {
            SamplerAggregatorConfig::NaiveMonteCarlo { materializer, .. }
            | SamplerAggregatorConfig::RasterPlane { materializer, .. }
            | SamplerAggregatorConfig::RasterLine { materializer, .. }
            | SamplerAggregatorConfig::PdfAdaptationRasterPlane { materializer, .. }
            | SamplerAggregatorConfig::PdfAdaptationRasterLine { materializer, .. }
            | SamplerAggregatorConfig::HavanaTraining { materializer, .. }
            | SamplerAggregatorConfig::HavanaInference { materializer, .. }
            | SamplerAggregatorConfig::ProcessSampler { materializer, .. } => materializer.as_ref(),
        }
    }
}
