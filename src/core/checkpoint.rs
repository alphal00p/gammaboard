use crate::evaluation::AccumulatorState;
use crate::sampling::SamplerAggregatorSnapshot;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) enum AccumulatorCheckpointState {
    #[default]
    NeedsInitialRoundTrip,
    WaitingForInitialRoundTrip,
    Ready,
}

/// Durable sampler progress. Live timing windows and pending writes are runner state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct SamplerProgress {
    pub produced_batches_total: i64,
    pub produced_samples_total: i64,
    pub ingested_batches_total: i64,
    pub ingested_samples_total: i64,
    pub sampler_uptime_ms_accumulated: f64,
    pub accumulator_checkpoint_state: AccumulatorCheckpointState,
}

/// The queue owns both recovery cursors and the adaptive batch size.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SamplerQueueCheckpoint {
    pub last_completed_batch_id: Option<i64>,
    pub last_produced_batch_id: Option<i64>,
    pub batch_size_current: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerAggregatorCheckpoint {
    pub completed_samples: i64,
    pub task_id: i64,
    pub(crate) output_snapshot_id: Option<i64>,
    pub(crate) batches_completed: i32,
    pub sampler_snapshot: SamplerAggregatorSnapshot,
    pub observable_state: AccumulatorState,
    pub(crate) runtime_state: SamplerProgress,
    pub(crate) queue: SamplerQueueCheckpoint,
}

impl SamplerAggregatorCheckpoint {
    pub(crate) fn produced_samples(&self) -> i64 {
        self.runtime_state.produced_samples_total
    }
}
