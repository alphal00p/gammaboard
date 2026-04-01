pub mod batch_transform;
pub(crate) mod havana_grid;
pub mod latent_batch;
pub mod materializer;
pub mod sampler;
pub mod sampler_aggregator;

use crate::evaluation::ObservableState;
use crate::{core::RunStageSnapshot, runners::sampler_aggregator::SamplerAggregatorCheckpoint};

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

impl From<RunStageSnapshot> for StageHandoffOwned {
    fn from(snapshot: RunStageSnapshot) -> Self {
        Self {
            sampler_snapshot: snapshot.sampler_snapshot,
            observable_state: snapshot.observable_state,
        }
    }
}

impl From<SamplerAggregatorCheckpoint> for StageHandoffOwned {
    fn from(snapshot: SamplerAggregatorCheckpoint) -> Self {
        Self {
            sampler_snapshot: Some(snapshot.sampler_snapshot),
            observable_state: Some(snapshot.observable_state),
        }
    }
}
