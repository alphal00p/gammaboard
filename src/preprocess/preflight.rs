use crate::core::{BuildError, RunStageSnapshot};

pub(super) fn build_initial_stage() -> Result<RunStageSnapshot, BuildError> {
    Ok(RunStageSnapshot {
        id: None,
        run_id: 0,
        task_id: None,
        name: "root".to_string(),
        sequence_nr: None,
        queue_empty: true,
        sampler_snapshot: None,
        observable_state: None,
        sampler_aggregator: None,
        batch_transforms: Vec::new(),
    })
}
