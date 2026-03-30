pub mod batch_transform;
pub mod latent_batch;
pub mod materializer;
pub mod sampler;
pub mod sampler_aggregator;

use crate::evaluation::ObservableState;

pub use batch_transform::{SphericalBatchTransformParams, UnitBallBatchTransformParams};
pub use latent_batch::{LatentBatch, LatentBatchPayload, LatentBatchSpec, SamplePlan};
pub use sampler::{SamplerAggregator, SamplerAggregatorSnapshot};
pub use sampler_aggregator::{
    HavanaInferenceSamplerParams, HavanaInferenceSource, HavanaSamplerParams,
    NaiveMonteCarloSamplerParams, RasterLineSamplerParams, RasterPlaneSamplerParams,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct StageHandoff<'a> {
    pub sampler_snapshot: Option<&'a SamplerAggregatorSnapshot>,
    pub observable_state: Option<&'a ObservableState>,
}

#[derive(Debug, Clone, Default)]
pub struct StageHandoffOwned {
    pub sampler_snapshot: Option<SamplerAggregatorSnapshot>,
    pub observable_state: Option<ObservableState>,
}

impl StageHandoffOwned {
    pub fn as_ref(&self) -> StageHandoff<'_> {
        StageHandoff {
            sampler_snapshot: self.sampler_snapshot.as_ref(),
            observable_state: self.observable_state.as_ref(),
        }
    }
}
