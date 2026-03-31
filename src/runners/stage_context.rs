use crate::api::stage::resolve_task_source_snapshot;
use crate::core::{AggregationStore, BatchTransformConfig, RunTask, RunTaskStore, StoreError};
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
        if snapshot
            .sampler_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.contains_havana_grid())
        {
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
        resolve_task_source_snapshot(store, run_id, task, task.task.sample_sampler_source())
            .await?;
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
                .and_then(|snapshot| snapshot.sampler_aggregator.clone())
        })
        .or_else(|| {
            base_stage_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.sampler_aggregator.clone())
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
