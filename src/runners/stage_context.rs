use crate::core::{
    AggregationStore, BatchTransformConfig, RunTask, RunTaskStore, SourceRefSpec, StoreError,
};
use crate::runners::sampler_aggregator::SamplerAggregatorRunnerSnapshot;
use crate::sampling::StageHandoffOwned;

pub(crate) const HAVANA_HANDOFF_REQUIRED_ERROR: &str =
    "havana_inference sampler requires a havana training or inference snapshot handoff";

pub(crate) struct ResolvedStageContext {
    pub(crate) sampler_config: crate::core::SamplerAggregatorConfig,
    pub(crate) batch_transforms: Vec<BatchTransformConfig>,
    pub(crate) handoff: Option<StageHandoffOwned>,
}

fn handoff_contains_havana_grid(handoff: &StageHandoffOwned) -> bool {
    handoff
        .sampler_snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.contains_havana_grid())
}

pub(crate) async fn resolve_source_snapshot<S>(
    store: &S,
    run_id: i32,
    task: &RunTask,
    source: Option<SourceRefSpec>,
) -> Result<Option<crate::core::RunStageSnapshot>, StoreError>
where
    S: AggregationStore + RunTaskStore + Send + Sync,
{
    match source {
        Some(SourceRefSpec::Latest) => {
            store
                .load_latest_stage_snapshot_before_sequence(run_id, task.sequence_nr)
                .await
        }
        Some(SourceRefSpec::FromName(source_task_name)) => {
            let source_task = store
                .list_run_tasks(run_id)
                .await?
                .into_iter()
                .find(|candidate| candidate.name == source_task_name)
                .ok_or_else(|| {
                    StoreError::store(format!(
                        "task {} references source task '{}' but no such task exists in run {}",
                        task.id, source_task_name, run_id
                    ))
                })?;
            if source_task.sequence_nr >= task.sequence_nr {
                return Err(StoreError::store(format!(
                    "task {} references source task '{}' which is not prior in sequence",
                    task.id, source_task_name
                )));
            }
            let snapshot = store
                .load_latest_stage_snapshot_before_sequence(run_id, source_task.sequence_nr + 1)
                .await?
                .ok_or_else(|| {
                    StoreError::store(format!(
                        "task {} source task '{}' has no queue-empty stage snapshot",
                        task.id, source_task_name
                    ))
                })?;
            if snapshot.task_id != Some(source_task.id) {
                return Err(StoreError::store(format!(
                    "task {} source task '{}' has no queue-empty stage snapshot",
                    task.id, source_task_name
                )));
            }
            Ok(Some(snapshot))
        }
        None => Ok(None),
    }
}

pub(crate) async fn find_latest_havana_snapshot_before_sequence<S>(
    store: &S,
    run_id: i32,
    sequence_nr: i32,
) -> Result<Option<crate::core::RunStageSnapshot>, StoreError>
where
    S: AggregationStore + Send + Sync,
{
    let mut search_seq = sequence_nr;
    loop {
        let Some(snapshot) = store
            .load_latest_stage_snapshot_before_sequence(run_id, search_seq)
            .await?
        else {
            return Ok(None);
        };
        if snapshot.sampler_snapshot.contains_havana_grid() {
            return Ok(Some(snapshot));
        }
        let prev_seq = snapshot.sequence_nr.unwrap_or(0);
        if prev_seq <= 0 {
            return Ok(None);
        }
        search_seq = prev_seq;
    }
}

pub(crate) async fn resolve_stage_context<S>(
    store: &S,
    run_id: i32,
    task: &RunTask,
    fallback_sequence_nr: i32,
    restored_snapshot: Option<SamplerAggregatorRunnerSnapshot>,
) -> Result<ResolvedStageContext, StoreError>
where
    S: AggregationStore + RunTaskStore + Send + Sync,
{
    let source_snapshot =
        resolve_source_snapshot(store, run_id, task, task.task.sample_sampler_source()).await?;
    let base_stage_snapshot = store
        .load_latest_stage_snapshot_before_sequence(run_id, fallback_sequence_nr)
        .await?;

    let sampler_config = task
        .task
        .sampler_config()
        .or_else(|| task.task.sample_sampler_config())
        .or_else(|| {
            source_snapshot
                .as_ref()
                .map(|snapshot| snapshot.sampler_aggregator.clone())
        })
        .or_else(|| {
            base_stage_snapshot
                .as_ref()
                .map(|snapshot| snapshot.sampler_aggregator.clone())
        })
        .ok_or_else(|| {
            StoreError::store(format!(
                "run {} task {} has no sampler configuration",
                run_id, task.id
            ))
        })?;

    let batch_transforms = task
        .task
        .batch_transforms_config()
        .or_else(|| {
            source_snapshot
                .as_ref()
                .map(|snapshot| snapshot.batch_transforms.clone())
        })
        .or_else(|| {
            base_stage_snapshot
                .as_ref()
                .map(|snapshot| snapshot.batch_transforms.clone())
        })
        .unwrap_or_default();

    let handoff = if let Some(snapshot) = restored_snapshot {
        Some(snapshot.into())
    } else if let Some(snapshot) = source_snapshot {
        Some(snapshot.into())
    } else {
        match &sampler_config {
            crate::core::SamplerAggregatorConfig::HavanaInference { params } => {
                let snapshot = match &params.source {
                    crate::sampling::HavanaInferenceSource::Snapshot { snapshot_id } => {
                        store.load_stage_snapshot(*snapshot_id).await?
                    }
                    crate::sampling::HavanaInferenceSource::LatestTrainingSamplerAggregator => {
                        find_latest_havana_snapshot_before_sequence(
                            store,
                            run_id,
                            fallback_sequence_nr,
                        )
                        .await?
                    }
                };
                match snapshot {
                    Some(snapshot) => Some(snapshot.into()),
                    None => {
                        return Err(StoreError::store(HAVANA_HANDOFF_REQUIRED_ERROR));
                    }
                }
            }
            _ => base_stage_snapshot.map(Into::into),
        }
    };

    let handoff = match (&sampler_config, handoff) {
        (crate::core::SamplerAggregatorConfig::HavanaInference { .. }, Some(handoff))
            if !handoff_contains_havana_grid(&handoff) =>
        {
            find_latest_havana_snapshot_before_sequence(store, run_id, fallback_sequence_nr)
                .await?
                .map(Into::into)
        }
        (_, handoff) => handoff,
    };

    Ok(ResolvedStageContext {
        sampler_config,
        batch_transforms,
        handoff,
    })
}
