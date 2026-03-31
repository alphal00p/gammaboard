use crate::core::{
    AggregationStore, ObservableConfig, RunStageSnapshot, RunTask, RunTaskStore, SourceRefSpec,
    StoreError,
};

pub async fn resolve_task_source_snapshot<S>(
    store: &S,
    run_id: i32,
    task: &RunTask,
    source: Option<SourceRefSpec>,
) -> Result<Option<RunStageSnapshot>, StoreError>
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

pub async fn resolve_effective_sample_observable_config<S>(
    store: &S,
    run_id: i32,
    task: &RunTask,
) -> Result<Option<ObservableConfig>, StoreError>
where
    S: AggregationStore + RunTaskStore + Send + Sync,
{
    if let Some(config) = task
        .task
        .new_observable_config()
        .map_err(|err| StoreError::store(err.to_string()))?
    {
        return Ok(Some(config));
    }

    if let Some(source_snapshot) =
        resolve_task_source_snapshot(store, run_id, task, task.task.sample_observable_source())
            .await?
    {
        if let Some(observable) = source_snapshot.observable_state {
            return Ok(Some(observable.config()));
        }
    }

    if let Some(base_snapshot) = store
        .load_latest_stage_snapshot_before_sequence(run_id, task.sequence_nr)
        .await?
    {
        if let Some(observable) = base_snapshot.observable_state {
            return Ok(Some(observable.config()));
        }
    }

    Ok(None)
}
