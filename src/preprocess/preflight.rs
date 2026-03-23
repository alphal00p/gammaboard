use std::collections::BTreeMap;

use crate::core::{
    BatchTransformConfig, BuildError, IntoPreflightTask, ObservableConfig, RunStageSnapshot,
    RunTaskSpec, SamplerAggregatorConfig,
};
use crate::evaluation::{Batch, EvalBatchOptions, Evaluator, ObservableState, PointSpec};
use crate::sampling::{SamplerAggregatorSnapshot, StageHandoff};

#[derive(Debug, Clone)]
struct PreflightStageState {
    sampler_snapshot: SamplerAggregatorSnapshot,
    observable_config: Option<ObservableConfig>,
}

impl PreflightStageState {
    fn handoff(&self) -> StageHandoff<'_> {
        StageHandoff {
            sampler_snapshot: Some(&self.sampler_snapshot),
            observable_state: None,
        }
    }

    fn from_snapshot(snapshot: &RunStageSnapshot) -> Self {
        Self {
            sampler_snapshot: snapshot.sampler_snapshot.clone(),
            observable_config: snapshot
                .observable_state
                .as_ref()
                .map(ObservableState::config),
        }
    }
}

pub(super) fn build_initial_stage(
    initial_sampler_aggregator: &SamplerAggregatorConfig,
    initial_batch_transforms: &[BatchTransformConfig],
    point_spec: &PointSpec,
) -> Result<(serde_json::Value, RunStageSnapshot), BuildError> {
    let mut sampler = initial_sampler_aggregator.build(point_spec.clone(), None, None)?;
    sampler.validate_point_spec(point_spec)?;
    let sampler_init_metadata = sampler.get_init_metadata();

    let materializer =
        initial_sampler_aggregator.build_materializer(point_spec.clone(), None, None)?;
    materializer.validate_point_spec(point_spec)?;
    for transform in initial_batch_transforms {
        transform.build()?.validate_point_spec(point_spec)?;
    }

    Ok((
        sampler_init_metadata,
        RunStageSnapshot {
            id: None,
            run_id: 0,
            task_id: None,
            sequence_nr: None,
            queue_empty: true,
            sampler_snapshot: sampler.snapshot()?,
            observable_state: None,
            sampler_aggregator: initial_sampler_aggregator.clone(),
            batch_transforms: initial_batch_transforms.to_vec(),
        },
    ))
}

pub fn preflight_task_suffix(
    base_snapshot: &RunStageSnapshot,
    referenced_snapshots: &BTreeMap<i64, RunStageSnapshot>,
    resolved_tasks: &[RunTaskSpec],
    evaluator: &mut dyn Evaluator,
    point_spec: &PointSpec,
) -> Result<(), BuildError> {
    let mut current_state = PreflightStageState::from_snapshot(base_snapshot);

    let preflight_tasks = resolved_tasks
        .iter()
        .cloned()
        .map(IntoPreflightTask::into_preflight)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    for task in preflight_tasks {
        if let Some(start_from) = task.start_from() {
            current_state = PreflightStageState::from_snapshot(
                referenced_snapshots
                    .get(&start_from.snapshot_id)
                    .ok_or_else(|| {
                        BuildError::build(format!(
                            "missing referenced stage snapshot {} during preflight",
                            start_from.snapshot_id
                        ))
                    })?,
            );
        }

        let (Some(sampler_aggregator), Some(batch_transforms)) =
            (task.sampler_config(), task.batch_transforms_config())
        else {
            continue;
        };
        let observable_config = task
            .new_observable_config()?
            .or_else(|| current_state.observable_config.clone())
            .ok_or_else(|| {
                BuildError::build(
                    "task requested observable reuse but no previous observable exists",
                )
            })?;
        current_state = if task.nr_expected_samples() == Some(0) {
            preflight_configure_stage(
                &sampler_aggregator,
                &batch_transforms,
                Some(current_state.handoff()),
                point_spec,
                Some(observable_config),
            )?
        } else {
            preflight_single_stage(
                task.nr_expected_samples()
                    .and_then(|n| usize::try_from(n).ok()),
                observable_config,
                &sampler_aggregator,
                &batch_transforms,
                evaluator,
                point_spec,
                Some(current_state.handoff()),
            )?
        };
    }

    Ok(())
}

fn preflight_configure_stage(
    sampler_aggregator: &SamplerAggregatorConfig,
    batch_transforms: &[BatchTransformConfig],
    handoff: Option<StageHandoff<'_>>,
    point_spec: &PointSpec,
    observable_config: Option<ObservableConfig>,
) -> Result<PreflightStageState, BuildError> {
    let mut sampler = sampler_aggregator.build(point_spec.clone(), None, handoff)?;
    sampler.validate_point_spec(point_spec)?;
    let mut materializer =
        sampler_aggregator.build_materializer(point_spec.clone(), None, handoff)?;
    materializer.validate_point_spec(point_spec)?;
    for transform in batch_transforms {
        transform.build()?.validate_point_spec(point_spec)?;
    }

    Ok(PreflightStageState {
        sampler_snapshot: sampler.snapshot()?,
        observable_config,
    })
}

fn preflight_single_stage(
    sample_budget: Option<usize>,
    task_observable: ObservableConfig,
    sampler_aggregator: &SamplerAggregatorConfig,
    batch_transforms: &[BatchTransformConfig],
    evaluator: &mut dyn Evaluator,
    point_spec: &PointSpec,
    handoff: Option<StageHandoff<'_>>,
) -> Result<PreflightStageState, BuildError> {
    let mut sampler = sampler_aggregator.build(point_spec.clone(), sample_budget, handoff)?;
    sampler.validate_point_spec(point_spec)?;

    let mut materializer =
        sampler_aggregator.build_materializer(point_spec.clone(), sample_budget, handoff)?;
    materializer.validate_point_spec(point_spec)?;
    let transforms = batch_transforms
        .iter()
        .map(|config| {
            let transform = config.build()?;
            transform.validate_point_spec(point_spec)?;
            Ok(transform)
        })
        .collect::<Result<Vec<_>, BuildError>>()?;

    let require_training_values = sampler.training_samples_remaining().is_some();
    let latent_batch = sampler
        .produce_latent_batch(1)
        .map_err(|err| {
            BuildError::incompatible(format!(
                "preflight failed to produce latent batch with sampler {}: {err}",
                sampler_aggregator.kind_str()
            ))
        })?
        .with_observable_config(task_observable.clone())
        .build();

    let materialized_batch = materializer
        .materialize_batch(&latent_batch)
        .map_err(|err| {
            BuildError::incompatible(format!(
                "preflight materializer materialization failed: {err}"
            ))
        })?;
    let transformed_batch =
        apply_batch_transforms(materialized_batch, &transforms).map_err(|err| {
            BuildError::incompatible(format!("preflight batch transform failed: {err}"))
        })?;
    transformed_batch
        .validate_point_spec(point_spec)
        .map_err(|err| {
            BuildError::incompatible(format!(
                "preflight transformed batch has invalid point_spec: {err}"
            ))
        })?;

    let result = evaluator
        .eval_batch(
            &transformed_batch,
            &latent_batch.observable,
            EvalBatchOptions {
                require_training_values,
            },
        )
        .map_err(|err| {
            BuildError::incompatible(format!("preflight evaluator dry-run failed: {err}"))
        })?;

    if require_training_values {
        let training_weights = result.values.as_deref().ok_or_else(|| {
            BuildError::incompatible(
                "preflight evaluator dry-run omitted training values while sampler training is active",
            )
        })?;
        sampler
            .ingest_training_weights(training_weights)
            .map_err(|err| {
                BuildError::incompatible(format!(
                    "preflight sampler ingest of evaluator training values failed: {err}"
                ))
            })?;
    }

    Ok(PreflightStageState {
        sampler_snapshot: sampler
            .snapshot()
            .map_err(|err| BuildError::build(format!("preflight snapshot failed: {err}")))?,
        observable_config: Some(task_observable),
    })
}

fn apply_batch_transforms(
    mut batch: Batch,
    transforms: &[Box<dyn crate::evaluation::BatchTransform>],
) -> Result<Batch, crate::core::EngineError> {
    for transform in transforms {
        batch = transform.apply(batch)?;
    }
    Ok(batch)
}
