use crate::core::SamplerAggregatorCheckpoint;
use crate::core::{
    AggregationStore, BatchTransformConfig, EvaluatorConfig, RunTask, RunTaskStore, StoreError,
};
use crate::sampling::{Sampler, SavedSampler, StageHandoff, StageHandoffOwned};
use crate::services::stage::resolve_task_source_snapshot;
use crate::{Domain, EngineError, Materializer, SamplingConfig};
use serde_json::Value as JsonValue;

pub(crate) const HAVANA_HANDOFF_REQUIRED_ERROR: &str =
    "havana_inference sampler requires a havana training or inference snapshot handoff";
pub(crate) const PDF_ADAPTATION_HANDOFF_REQUIRED_ERROR: &str =
    "pdf_adaptation sampler requires a persisted sampler snapshot handoff";

pub(crate) struct ResolvedStageContext {
    pub(crate) evaluator_config: EvaluatorConfig,
    pub(crate) evaluator_provenance: StageConfigProvenance,
    pub(crate) sampler: SamplerInitialization,
    pub(crate) sampler_provenance: StageConfigProvenance,
    pub(crate) batch_transforms: Vec<BatchTransformConfig>,
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

/// One initialization decision, resolved before starting either runtime role.
pub(crate) enum SamplerInitialization {
    Fresh(SamplingConfig),
    Restore(SavedSampler),
    FromStage {
        config: SamplingConfig,
        source: Box<StageHandoffOwned>,
    },
}

impl SamplerInitialization {
    pub(crate) fn config(&self) -> &SamplingConfig {
        match self {
            Self::Fresh(config) | Self::FromStage { config, .. } => config,
            Self::Restore(saved) => &saved.config,
        }
    }

    pub(crate) fn initialize(
        self,
        domain: Domain,
        sample_budget: Option<usize>,
        metadata: JsonValue,
    ) -> Result<Box<dyn Sampler>, EngineError> {
        match self {
            Self::Fresh(config) => config.create(domain, sample_budget, metadata),
            Self::Restore(saved) => saved.restore(&domain, metadata),
            Self::FromStage { config, source } => {
                config.from_stage(&domain, source.as_ref().as_ref())
            }
        }
    }

    pub(crate) fn build_materializer(
        &self,
        domain: &Domain,
    ) -> Result<Box<dyn Materializer>, EngineError> {
        let source = match self {
            Self::Fresh(_) => None,
            Self::Restore(saved) => Some(StageHandoff {
                sampler_snapshot: Some(saved),
                observable_state: None,
            }),
            Self::FromStage { source, .. } => Some(source.as_ref().as_ref()),
        };
        self.config().build_materializer(domain, source)
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
        .filter(|snapshot| snapshot.sampler_config().is_some())
    {
        (
            snapshot.sampler_config().expect("checked above").clone(),
            StageConfigProvenance::from_snapshot(snapshot),
        )
    } else if let Some(snapshot) = base_stage_snapshot
        .as_ref()
        .filter(|snapshot| snapshot.sampler_config().is_some())
    {
        (
            snapshot.sampler_config().expect("checked above").clone(),
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

    // A checkpoint owns the exact configuration as well as sampler state.
    // A new explicit configuration always creates new state or performs a
    // supported handoff; equality of sampler kinds never selects restoration.
    let sampler = if let Some(checkpoint) = restored_snapshot {
        SamplerInitialization::Restore(checkpoint.sampler_snapshot)
    } else if !has_explicit_sampler_config
        && let Some(saved) = sampler_source_snapshot
            .as_ref()
            .or(base_stage_snapshot.as_ref())
            .and_then(|stage| stage.sampler_snapshot.clone())
    {
        SamplerInitialization::Restore(saved)
    } else if matches!(
        &task.task,
        crate::core::RunTaskSpec::Sample {
            sampler: Some(crate::core::tasks::StageSourceSpec::FromStage { .. }),
            ..
        }
    ) {
        let source = sampler_source_snapshot
            .ok_or_else(|| StoreError::store("sampler handoff stage is missing"))?;
        if source.sampler_snapshot.is_none() {
            return Err(StoreError::store(
                "sampler handoff stage has no saved sampler",
            ));
        }
        SamplerInitialization::FromStage {
            config: sampler_config,
            source: Box::new(source.into()),
        }
    } else {
        match &sampler_config {
            SamplingConfig {
                sampler: crate::core::SamplerConfig::HavanaInference { .. },
                ..
            } => {
                let source = if let Some(source) = sampler_source_snapshot {
                    Some(source)
                } else {
                    find_latest_havana_snapshot_before_sequence(store, run_id, fallback_sequence_nr)
                        .await?
                }
                .ok_or_else(|| StoreError::store(HAVANA_HANDOFF_REQUIRED_ERROR))?;
                if !source
                    .sampler_snapshot
                    .as_ref()
                    .is_some_and(SavedSampler::contains_havana_grid)
                {
                    return Err(StoreError::store(HAVANA_HANDOFF_REQUIRED_ERROR));
                }
                SamplerInitialization::FromStage {
                    config: sampler_config,
                    source: Box::new(source.into()),
                }
            }
            SamplingConfig {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterPlane { .. },
                ..
            }
            | SamplingConfig {
                sampler: crate::core::SamplerConfig::PdfAdaptationRasterLine { .. },
                ..
            } => {
                let source = sampler_source_snapshot
                    .or(base_stage_snapshot)
                    .filter(|stage| stage.sampler_snapshot.is_some())
                    .ok_or_else(|| StoreError::store(PDF_ADAPTATION_HANDOFF_REQUIRED_ERROR))?;
                SamplerInitialization::FromStage {
                    config: sampler_config,
                    source: Box::new(source.into()),
                }
            }
            _ => SamplerInitialization::Fresh(sampler_config),
        }
    };

    Ok(ResolvedStageContext {
        evaluator_config,
        evaluator_provenance,
        sampler,
        sampler_provenance,
        batch_transforms,
    })
}
