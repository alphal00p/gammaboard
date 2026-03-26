use crate::core::{BatchTransformConfig, BuildError, RunStageSnapshot, SamplerAggregatorConfig};
use crate::evaluation::PointSpec;

/// New variant which accepts an optional `sample_budget`. When creating an initial stage for
/// a sampler that requires a training budget (e.g. `HavanaTraining`), callers can pass the
/// intended budget so the sampler can be constructed properly for initial snapshot construction.
pub(super) fn build_initial_stage_with_budget(
    initial_sampler_aggregator: &SamplerAggregatorConfig,
    initial_batch_transforms: &[BatchTransformConfig],
    point_spec: &PointSpec,
    sample_budget: Option<usize>,
) -> Result<RunStageSnapshot, BuildError> {
    // Pass the provided sample_budget into the sampler builder. The handoff is None for initial stage.
    let mut sampler = initial_sampler_aggregator.build(point_spec.clone(), sample_budget, None)?;
    sampler.validate_point_spec(point_spec)?;
    let materializer =
        initial_sampler_aggregator.build_materializer(point_spec.clone(), None, None)?;
    materializer.validate_point_spec(point_spec)?;
    for transform in initial_batch_transforms {
        transform.build()?.validate_point_spec(point_spec)?;
    }

    Ok(RunStageSnapshot {
        id: None,
        run_id: 0,
        task_id: None,
        name: "root".to_string(),
        sequence_nr: None,
        queue_empty: true,
        sampler_snapshot: sampler.snapshot()?,
        observable_state: None,
        sampler_aggregator: initial_sampler_aggregator.clone(),
        batch_transforms: initial_batch_transforms.to_vec(),
    })
}
