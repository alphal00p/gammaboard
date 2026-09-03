use crate::api::stage::resolve_task_source_snapshot;
use crate::core::{
    AggregationStore, BatchTransformConfig, EvaluatorConfig, RunTask, RunTaskStore, StoreError,
};
use crate::runners::sampler_aggregator::SamplerAggregatorCheckpoint;
use crate::sampling::StageHandoffOwned;

pub(crate) const HAVANA_HANDOFF_REQUIRED_ERROR: &str =
    "havana_inference sampler requires a havana training or inference snapshot handoff";
pub(crate) const PDF_ADAPTATION_HANDOFF_REQUIRED_ERROR: &str =
    "pdf_adaptation sampler requires a persisted sampler snapshot handoff";

pub(crate) struct ResolvedStageContext {
    pub(crate) evaluator_config: EvaluatorConfig,
    pub(crate) evaluator_provenance: StageConfigProvenance,
    pub(crate) sampler_config: crate::core::SamplerAggregatorConfig,
    pub(crate) sampler_provenance: StageConfigProvenance,
    pub(crate) batch_transforms: Vec<BatchTransformConfig>,
    pub(crate) handoff: Option<StageHandoffOwned>,
}

#[derive(Debug, Clone)]
pub(crate) struct StageConfigProvenance {
    pub(crate) name: String,
    pub(crate) task_id: Option<i64>,
    pub(crate) snapshot_id: Option<i64>,
}

impl StageConfigProvenance {
    fn from_task(task: &RunTask) -> Self {
        Self {
            name: task.name.clone(),
            task_id: Some(task.id),
            snapshot_id: None,
        }
    }

    pub(crate) fn from_snapshot(snapshot: &crate::core::RunStageSnapshot) -> Self {
        Self {
            name: snapshot.name.clone(),
            task_id: snapshot.task_id,
            snapshot_id: snapshot.id,
        }
    }
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
    restored_snapshot: Option<SamplerAggregatorCheckpoint>,
) -> Result<ResolvedStageContext, StoreError>
where
    S: AggregationStore + RunTaskStore + Send + Sync,
{
    let has_explicit_sampler_config =
        task.task.sampler_config().is_some() || task.task.sample_sampler_config().is_some();
    let restoring_active_task = restored_snapshot.is_some();
    let sampler_source_snapshot =
        resolve_task_source_snapshot(store, run_id, task, task.task.sample_sampler_source())
            .await?;
    let evaluator_source_snapshot =
        resolve_task_source_snapshot(store, run_id, task, task.task.evaluator_source()).await?;
    let base_stage_snapshot = store
        .load_latest_stage_snapshot_before_sequence(run_id, fallback_sequence_nr)
        .await?;

    let (evaluator_config, evaluator_provenance) =
        if let Some(config) = task.task.evaluator_config() {
            (config, StageConfigProvenance::from_task(task))
        } else if let Some(snapshot) = evaluator_source_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.evaluator.is_some())
        {
            (
                snapshot.evaluator.clone().expect("checked above"),
                StageConfigProvenance::from_snapshot(snapshot),
            )
        } else if let Some(snapshot) = base_stage_snapshot
            .as_ref()
            .filter(|snapshot| snapshot.evaluator.is_some())
        {
            (
                snapshot.evaluator.clone().expect("checked above"),
                StageConfigProvenance::from_snapshot(snapshot),
            )
        } else {
            return Err(StoreError::store(format!(
                "run {} task {} has no evaluator configuration",
                run_id, task.id
            )));
        };

    let task_sampler_config = task
        .task
        .sampler_config()
        .or_else(|| task.task.sample_sampler_config());
    let (sampler_config, sampler_provenance) = if let Some(config) = task_sampler_config {
        (config, StageConfigProvenance::from_task(task))
    } else if let Some(snapshot) = sampler_source_snapshot
        .as_ref()
        .filter(|snapshot| snapshot.sampler_aggregator.is_some())
    {
        (
            snapshot.sampler_aggregator.clone().expect("checked above"),
            StageConfigProvenance::from_snapshot(snapshot),
        )
    } else if let Some(snapshot) = base_stage_snapshot
        .as_ref()
        .filter(|snapshot| snapshot.sampler_aggregator.is_some())
    {
        (
            snapshot.sampler_aggregator.clone().expect("checked above"),
            StageConfigProvenance::from_snapshot(snapshot),
        )
    } else {
        return Err(StoreError::store(format!(
            "run {} task {} has no sampler configuration",
            run_id, task.id
        )));
    };

    let batch_transforms = task
        .task
        .batch_transforms_config()
        .or_else(|| {
            sampler_source_snapshot
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
    } else if let Some(snapshot) = sampler_source_snapshot {
        Some(snapshot.into())
    } else {
        match &sampler_config {
            crate::core::SamplerAggregatorConfig::HavanaInference { params, .. } => {
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
            crate::core::SamplerAggregatorConfig::PdfAdaptationRasterPlane { .. }
            | crate::core::SamplerAggregatorConfig::PdfAdaptationRasterLine { .. } => {
                if let Some(snapshot) = base_stage_snapshot.clone() {
                    Some(snapshot.into())
                } else {
                    return Err(StoreError::store(PDF_ADAPTATION_HANDOFF_REQUIRED_ERROR));
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
    let handoff = if has_explicit_sampler_config
        && !restoring_active_task
        && !matches!(
            sampler_config,
            crate::core::SamplerAggregatorConfig::HavanaInference { .. }
                | crate::core::SamplerAggregatorConfig::PdfAdaptationRasterPlane { .. }
                | crate::core::SamplerAggregatorConfig::PdfAdaptationRasterLine { .. }
        ) {
        None
    } else {
        handoff
    };

    Ok(ResolvedStageContext {
        evaluator_config,
        evaluator_provenance,
        sampler_config,
        sampler_provenance,
        batch_transforms,
        handoff,
    })
}
