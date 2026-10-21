mod havana;
mod naive_monte_carlo;
mod process;
mod raster;

use crate::Materializer;
use crate::core::{EngineError, MaterializerConfig, SamplingConfig};
use crate::evaluation::AccumulatorState;
use crate::sampling::materializer::{
    HavanaInferenceMaterializer, IdentityMaterializer, ProcessMaterializer,
};
use crate::sampling::{Sampler, SavedSampler, StageHandoff};
use crate::utils::domain::Domain;
use serde_json::Value as JsonValue;

pub use self::havana::HavanaInferenceSamplerParams;
pub use self::havana::HavanaSamplerParams;
use self::havana::{
    HavanaInferenceSampler, HavanaInferenceSamplerSnapshot, HavanaSampler, HavanaSamplerSnapshot,
};
use self::naive_monte_carlo::NaiveMonteCarloSamplerAggregator;
pub use self::naive_monte_carlo::NaiveMonteCarloSamplerParams;
use self::process::ProcessSampler;
pub use self::raster::{
    PdfAdaptationImagePersistedOutput, RasterLineSamplerParams, RasterPlaneSamplerParams,
};
use self::raster::{
    PdfAdaptationRasterLineSampler, PdfAdaptationRasterPlaneSampler, PdfAdaptationRasterSnapshot,
    RasterLineSampler, RasterPlaneSampler, RasterSamplerSnapshot,
};
pub(crate) use self::raster::{coprime_stride, permuted_raster_index};

fn global_abs_integrand_norm_from_handoff(handoff: Option<StageHandoff<'_>>) -> Option<f64> {
    let observable = handoff.and_then(|value| value.observable_state)?;
    match observable {
        AccumulatorState::Vector(state) => {
            let value = state.projection().state.mean_abs();
            (value.is_finite() && value > 0.0).then_some(value)
        }
        _ => None,
    }
}

impl SamplingConfig {
    pub(crate) fn restore_state(
        &self,
        raw: JsonValue,
        domain: &Domain,
        evaluator_metadata: JsonValue,
    ) -> Result<Box<dyn Sampler>, EngineError> {
        match self {
            Self {
                sampler: crate::core::SamplerConfig::NaiveMonteCarlo { params, .. },
                ..
            } => {
                let snapshot: NaiveMonteCarloSamplerAggregator = serde_json::from_value(raw)
                    .map_err(|err| {
                        EngineError::build(format!(
                            "failed to decode naive_monte_carlo sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(NaiveMonteCarloSamplerAggregator::from_snapshot(
                    params.clone(),
                    snapshot,
                    domain,
                )?))
            }
            Self {
                sampler: crate::core::SamplerConfig::RasterPlane { params, .. },
                ..
            } => {
                let snapshot: RasterSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        EngineError::build(format!(
                            "failed to decode raster plane sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(RasterPlaneSampler::from_snapshot(
                    params.clone(),
                    snapshot,
                    domain,
                )?))
            }
            Self {
                sampler: crate::core::SamplerConfig::RasterLine { params, .. },
                ..
            } => {
                let snapshot: RasterSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        EngineError::build(format!(
                            "failed to decode raster line sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(RasterLineSampler::from_snapshot(
                    params.clone(),
                    snapshot,
                    domain,
                )?))
            }
            Self {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterPlane { params, .. },
                ..
            } => {
                let snapshot: PdfAdaptationRasterSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        EngineError::build(format!(
                            "failed to decode pdf adaptation raster plane sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(PdfAdaptationRasterPlaneSampler::from_snapshot(
                    RasterPlaneSamplerParams {
                        geometry: params.geometry.clone(),
                    },
                    snapshot,
                    domain,
                )?))
            }
            Self {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterLine { params, .. },
                ..
            } => {
                let snapshot: PdfAdaptationRasterSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        EngineError::build(format!(
                            "failed to decode pdf adaptation raster line sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(PdfAdaptationRasterLineSampler::from_snapshot(
                    RasterLineSamplerParams {
                        geometry: params.geometry.clone(),
                    },
                    snapshot,
                    domain,
                )?))
            }
            Self {
                sampler: crate::core::SamplerConfig::HavanaTraining { params, .. },
                ..
            } => {
                let snapshot: HavanaSamplerSnapshot =
                    serde_json::from_value(raw).map_err(|err| {
                        EngineError::build(format!(
                            "failed to decode havana sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(HavanaSampler::from_snapshot(
                    params.clone(),
                    snapshot,
                    domain,
                )?))
            }
            Self {
                sampler: crate::core::SamplerConfig::HavanaInference { .. },
                ..
            } => {
                let snapshot: HavanaInferenceSamplerSnapshot = serde_json::from_value(raw)
                    .map_err(|err| {
                        EngineError::build(format!(
                            "failed to decode havana inference sampler snapshot: {err}"
                        ))
                    })?;
                Ok(Box::new(HavanaInferenceSampler::from_snapshot(
                    snapshot, domain,
                )?))
            }
            Self {
                sampler: crate::core::SamplerConfig::ProcessSampler { params, .. },
                ..
            } => Ok(Box::new(ProcessSampler::from_snapshot(
                params.clone(),
                raw,
                domain,
                evaluator_metadata,
            )?)),
        }
    }
}

impl SamplingConfig {
    pub fn snapshot(
        &self,
        sampler: &mut dyn Sampler,
    ) -> Result<SavedSampler, crate::core::EngineError> {
        let state = sampler.snapshot()?;
        if state.is_null() {
            return Err(crate::core::EngineError::engine(
                "sampler snapshot must not be null",
            ));
        }
        Ok(SavedSampler {
            version: crate::compatibility::MODEL_VERSION,
            config: self.clone(),
            state,
        })
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self {
                sampler: crate::core::SamplerConfig::NaiveMonteCarlo { .. },
                ..
            } => "naive_monte_carlo",
            Self {
                sampler: crate::core::SamplerConfig::RasterPlane { .. },
                ..
            } => "raster_plane",
            Self {
                sampler: crate::core::SamplerConfig::RasterLine { .. },
                ..
            } => "raster_line",
            Self {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterPlane { .. },
                ..
            } => "pdf_adaptation_raster_plane",
            Self {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterLine { .. },
                ..
            } => "pdf_adaptation_raster_line",
            Self {
                sampler: crate::core::SamplerConfig::HavanaTraining { .. },
                ..
            } => "havana_training",
            Self {
                sampler: crate::core::SamplerConfig::HavanaInference { .. },
                ..
            } => "havana_inference",
            Self {
                sampler: crate::core::SamplerConfig::ProcessSampler { .. },
                ..
            } => "process_sampler",
        }
    }

    /// Construct fresh state using exactly this configuration.
    pub fn create(
        &self,
        domain: Domain,
        sample_budget: Option<usize>,
        evaluator_metadata: JsonValue,
    ) -> Result<Box<dyn Sampler>, EngineError> {
        match self {
            Self {
                sampler: crate::core::SamplerConfig::NaiveMonteCarlo { params, .. },
                ..
            } => Ok(Box::new(
                NaiveMonteCarloSamplerAggregator::from_params_and_domain(params.clone(), &domain)?,
            )),
            Self {
                sampler: crate::core::SamplerConfig::RasterPlane { params, .. },
                ..
            } => Ok(Box::new(RasterPlaneSampler::from_params_and_domain(
                params.clone(),
                &domain,
            )?)),
            Self {
                sampler: crate::core::SamplerConfig::RasterLine { params, .. },
                ..
            } => Ok(Box::new(RasterLineSampler::from_params_and_domain(
                params.clone(),
                &domain,
            )?)),
            Self {
                sampler: crate::core::SamplerConfig::HavanaTraining { params, .. },
                ..
            } => {
                let sample_budget = sample_budget.ok_or_else(|| {
                    EngineError::build("havana_training sampler requires a sample budget")
                })?;
                Ok(Box::new(HavanaSampler::from_params_and_domain(
                    params.clone(),
                    &domain,
                    sample_budget,
                )?))
            }
            Self {
                sampler: crate::core::SamplerConfig::HavanaInference { .. },
                ..
            }
            | Self {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterPlane { .. },
                ..
            }
            | Self {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterLine { .. },
                ..
            } => Err(EngineError::build(
                "this sampler requires explicit initialization from a stage",
            )),
            Self {
                sampler: crate::core::SamplerConfig::ProcessSampler { params, .. },
                ..
            } => Ok(Box::new(ProcessSampler::from_params_and_domain(
                params.clone(),
                &domain,
                evaluator_metadata,
            )?)),
        }
    }
    /// Initialize a new sampler from a supported source stage. Destination
    /// parameters apply; source task counters and pending work are not restored.
    pub fn from_stage(
        &self,
        domain: &Domain,
        source: StageHandoff<'_>,
    ) -> Result<Box<dyn Sampler>, EngineError> {
        let snapshot = source
            .sampler_snapshot
            .ok_or_else(|| EngineError::build("stage handoff requires a saved sampler"))?;
        match self {
            Self {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterPlane { params, .. },
                ..
            } => {
                let global_abs_integrand_norm =
                    global_abs_integrand_norm_from_handoff(Some(source));
                Ok(Box::new(
                    PdfAdaptationRasterPlaneSampler::from_params_and_snapshot(
                        crate::sampling::RasterPlaneSamplerParams {
                            geometry: params.geometry.clone(),
                        },
                        snapshot.clone(),
                        global_abs_integrand_norm,
                        domain,
                    )?,
                ))
            }
            Self {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterLine { params, .. },
                ..
            } => {
                let global_abs_integrand_norm =
                    global_abs_integrand_norm_from_handoff(Some(source));
                Ok(Box::new(
                    PdfAdaptationRasterLineSampler::from_params_and_snapshot(
                        crate::sampling::RasterLineSamplerParams {
                            geometry: params.geometry.clone(),
                        },
                        snapshot.clone(),
                        global_abs_integrand_norm,
                        domain,
                    )?,
                ))
            }
            Self {
                sampler: crate::core::SamplerConfig::HavanaInference { params, .. },
                ..
            } => Ok(Box::new(HavanaInferenceSampler::from_params_and_snapshot(
                params.clone(),
                snapshot.clone(),
                domain,
            )?)),
            _ => Err(EngineError::build(format!(
                "{} does not support stage handoff; create fresh state or restore saved state",
                self.kind_str(),
            ))),
        }
    }

    pub fn build_materializer(
        &self,
        domain: &Domain,
        handoff: Option<StageHandoff<'_>>,
    ) -> Result<Box<dyn Materializer>, EngineError> {
        if let Some(materializer) = self.materializer_config() {
            return match materializer {
                MaterializerConfig::ProcessMaterializer { params } => Ok(Box::new(
                    ProcessMaterializer::from_params_and_domain(params.clone(), domain)?,
                )),
            };
        }
        Ok(match self {
            SamplingConfig {
                sampler: crate::core::SamplerConfig::NaiveMonteCarlo { params, .. },
                ..
            } => Box::new(IdentityMaterializer::new_with_failure(
                params.fail_on_materialize_batch_nr,
            )),
            SamplingConfig {
                sampler: crate::core::SamplerConfig::HavanaInference { .. },
                ..
            } => Box::new(HavanaInferenceMaterializer::new(handoff)?),
            _ => Box::new(IdentityMaterializer::new()),
        })
    }

    fn materializer_config(&self) -> Option<&MaterializerConfig> {
        self.materializer.as_ref()
    }
}
