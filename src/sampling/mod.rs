pub mod latent_batch;
pub mod parametrization;
pub mod sampler;
pub mod sampler_aggregator;

use crate::evaluation::ObservableState;

pub use latent_batch::{LatentBatch, LatentBatchPayload, LatentBatchSpec, SamplePlan};
pub use parametrization::{
    HavanaInferenceParametrizationParams, IdentityParametrizationParams, ParametrizationSnapshot,
    SphericalParametrizationParams, UnitBallParametrizationParams,
};
pub use sampler::{SamplerAggregator, SamplerAggregatorSnapshot};
pub use sampler_aggregator::{
    HavanaInferenceSamplerParams, HavanaSamplerParams, NaiveMonteCarloSamplerParams,
    RasterLineSamplerParams, RasterPlaneSamplerParams,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct StageHandoff<'a> {
    pub sampler_snapshot: Option<&'a SamplerAggregatorSnapshot>,
    pub parametrization_snapshot: Option<&'a ParametrizationSnapshot>,
    pub observable_state: Option<&'a ObservableState>,
}
