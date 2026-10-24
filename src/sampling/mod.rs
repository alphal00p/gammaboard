pub(crate) mod batch_transform;
mod feedback;
pub(crate) mod havana_grid;
pub(crate) mod latent_batch;
pub(crate) mod materializer;
pub(crate) mod sampler;
pub(crate) mod sampler_aggregator;

use crate::core::{RunStageSnapshot, SamplerAggregatorCheckpoint};
use crate::evaluation::AccumulatorState;

pub use batch_transform::{SphericalBatchTransformParams, UnitBallBatchTransformParams};
pub use feedback::{Feedback, StratumAssignments};
pub use latent_batch::{LatentBatch, LatentBatchPayload};
pub use materializer::{HavanaInferenceMaterializer, IdentityMaterializer, ProcessMaterializer};
pub use sampler::{DiscreteSubspace, Generation, PdfPoint, Sampler, SavedSampler};
pub use sampler_aggregator::{
    HavanaInferenceSamplerParams, HavanaSamplerParams, NaiveMonteCarloSamplerParams,
    PdfAdaptationImagePersistedOutput, RasterLineSamplerParams, RasterPlaneSamplerParams,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct StageHandoff<'a> {
    pub sampler_snapshot: Option<&'a SavedSampler>,
    pub observable_state: Option<&'a AccumulatorState>,
}

#[derive(Debug, Clone, Default)]
pub struct StageHandoffOwned {
    pub sampler_snapshot: Option<SavedSampler>,
    pub observable_state: Option<AccumulatorState>,
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
pub(crate) mod generation;
#[cfg(test)]
mod stratification_tests;
