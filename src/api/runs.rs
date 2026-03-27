use crate::api::ApiError;
use crate::core::{
    AggregationStore, ControlPlaneStore, IntegrationParams, RunStageSnapshot, RunTask,
    RunTaskInput, RunTaskStore,
};
use crate::preprocess::{RunAddConfig, preprocess_run_add};
use crate::stores::RunProgress;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const DEFAULT_RUN_CONFIG_PATH: &str = "configs/default.toml";

#[derive(Debug, Clone)]
pub struct CreatedRun {
    pub run_id: i32,
    pub run_name: String,
    pub tasks_created: usize,
}

#[derive(Debug, Clone)]
pub struct ClonedRun {
    pub run_id: i32,
    pub run_name: String,
    pub source_run_id: i32,
    pub from_snapshot_id: i64,
    pub cloned_tasks: usize,
}

#[derive(Debug, Clone)]
pub struct AppendedTasks {
    pub tasks: Vec<RunTask>,
}

#[derive(Debug, Clone)]
pub struct PausedRun {
    pub run_id: i32,
    pub run_name: String,
    pub assignments_cleared: u64,
}

#[derive(Debug, Clone)]
pub struct RemovedRun {
    pub run_id: i32,
    pub run_name: String,
}

#[derive(Debug, Clone)]
pub struct RemovedPendingTask {
    pub run_id: i32,
    pub task_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskQueueFile {
    #[serde(default)]
    pub task: Option<RunTaskInput>,
    #[serde(default)]
    pub task_queue: Option<Vec<RunTaskInput>>,
}

impl TaskQueueFile {
    pub fn into_tasks(self) -> Vec<RunTaskInput> {
        let mut tasks = Vec::new();
        if let Some(task) = self.task {
            tasks.push(task);
        }
        if let Some(task_queue) = self.task_queue {
            tasks.extend(task_queue);
        }
        tasks
    }
}

pub fn parse_run_add_config_toml(raw: &str) -> Result<RunAddConfig, ApiError> {
    let mut merged = read_default_run_add_toml()?;
    let overlay = toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("failed parsing run TOML: {err}")))?;
    merge_toml(&mut merged, overlay);
    parse_run_add_config_value(merged)
}

pub fn load_run_add_config_file(path: &Path) -> Result<RunAddConfig, ApiError> {
    let default_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_RUN_CONFIG_PATH);
    let mut merged = read_toml_file(&default_path, "default run config")?;
    let overlay = read_toml_file(path, "run-add TOML")?;
    merge_toml(&mut merged, overlay);
    parse_run_add_config_value(merged)
}

pub fn parse_task_queue_toml(raw: &str) -> Result<TaskQueueFile, ApiError> {
    toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("invalid run-task payload: {err}")))
}

pub fn load_task_queue_file(path: &Path) -> Result<TaskQueueFile, ApiError> {
    read_toml_file(path, "run-task TOML")?
        .try_into()
        .map_err(|err| ApiError::BadRequest(format!("invalid run-task payload: {err}")))
}

pub async fn create_run(
    store: &(impl ControlPlaneStore + AggregationStore + RunTaskStore),
    config: RunAddConfig,
) -> Result<CreatedRun, ApiError> {
    let processed = preprocess_run_add(config)?;
    let point_spec = processed.point_spec.as_ref().ok_or_else(|| {
        ApiError::Internal("preprocessing did not resolve point_spec".to_string())
    })?;
    let resolved_integration_params =
        processed
            .resolved_integration_params
            .as_ref()
            .ok_or_else(|| {
                ApiError::Internal("preprocessing did not resolve integration_params".to_string())
            })?;
    let integration_params = serde_json::to_value(resolved_integration_params).map_err(|err| {
        ApiError::Internal(format!("failed to serialize integration_params: {err}"))
    })?;
    let initial_tasks = processed.resolved_task_queue.clone().unwrap_or_default();
    let initial_stage_snapshot = processed.initial_stage_snapshot.as_ref().ok_or_else(|| {
        ApiError::Internal("preprocessing did not build initial stage snapshot".to_string())
    })?;

    preflight_task_batch(store, initial_stage_snapshot.run_id, &initial_tasks).await?;

    let run_id = store
        .create_run(
            &processed.name,
            &integration_params,
            processed.target.as_ref(),
            point_spec,
            initial_stage_snapshot,
            &initial_tasks,
        )
        .await?;

    Ok(CreatedRun {
        run_id,
        run_name: processed.name,
        tasks_created: initial_tasks.len(),
    })
}

pub async fn clone_run(
    store: &(impl ControlPlaneStore + AggregationStore + RunTaskStore + crate::core::RunReadStore),
    source_run_id: i32,
    from_snapshot_id: i64,
    new_name: &str,
) -> Result<ClonedRun, ApiError> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err(ApiError::BadRequest(
            "invalid run name (`new_name`): expected non-empty string".to_string(),
        ));
    }

    let source_run = load_run_progress(store, source_run_id).await?;
    let point_spec = source_run.point_spec.clone().ok_or_else(|| {
        ApiError::Internal(format!("source run {source_run_id} is missing point_spec"))
    })?;
    let integration_params = source_run.integration_params.clone().ok_or_else(|| {
        ApiError::Internal(format!(
            "source run {source_run_id} is missing integration_params"
        ))
    })?;

    let snapshot = store
        .load_stage_snapshot(from_snapshot_id)
        .await?
        .ok_or_else(|| {
            ApiError::BadRequest(format!(
                "cannot clone from snapshot {from_snapshot_id}: no stage snapshot exists"
            ))
        })?;
    if snapshot.run_id != source_run_id {
        return Err(ApiError::BadRequest(format!(
            "snapshot {from_snapshot_id} belongs to run {}, not source run {source_run_id}",
            snapshot.run_id
        )));
    }

    let source_tasks = store.list_run_tasks(source_run_id).await?;
    let root_snapshot_name =
        format_clone_root_snapshot_name(&source_run.run_name, &source_tasks, &snapshot);
    let cloned_tasks: Vec<RunTaskInput> = Vec::new();
    let run_id = store
        .create_run(
            new_name,
            &integration_params,
            source_run.target.as_ref(),
            &point_spec,
            &RunStageSnapshot {
                id: None,
                run_id: 0,
                task_id: None,
                name: root_snapshot_name,
                sequence_nr: Some(0),
                queue_empty: snapshot.queue_empty,
                sampler_snapshot: snapshot.sampler_snapshot.clone(),
                observable_state: snapshot.observable_state.clone(),
                sampler_aggregator: snapshot.sampler_aggregator.clone(),
                batch_transforms: snapshot.batch_transforms.clone(),
            },
            &cloned_tasks,
        )
        .await?;

    Ok(ClonedRun {
        run_id,
        run_name: new_name.to_string(),
        source_run_id,
        from_snapshot_id,
        cloned_tasks: cloned_tasks.len(),
    })
}

pub async fn append_tasks(
    store: &(impl AggregationStore + crate::core::RunReadStore + RunTaskStore),
    run_id: i32,
    task_file: TaskQueueFile,
) -> Result<AppendedTasks, ApiError> {
    let tasks = task_file.into_tasks();
    let run = load_run_progress(store, run_id).await?;
    if run.integration_params.is_none() {
        return Err(ApiError::Internal(format!(
            "run {run_id} is missing integration_params"
        )));
    }
    preflight_task_batch(store, run_id, &tasks).await?;
    let tasks = store.append_run_tasks(run_id, &tasks).await?;
    Ok(AppendedTasks { tasks })
}

pub async fn pause_run(
    store: &(impl ControlPlaneStore + crate::core::RunReadStore),
    run_id: i32,
) -> Result<PausedRun, ApiError> {
    let run = load_run_progress(store, run_id).await?;
    let assignments_cleared = store.clear_desired_assignments_for_run(run_id).await?;
    Ok(PausedRun {
        run_id,
        run_name: run.run_name,
        assignments_cleared,
    })
}

pub async fn remove_run(
    store: &(impl ControlPlaneStore + crate::core::RunReadStore),
    run_id: i32,
) -> Result<RemovedRun, ApiError> {
    let run = load_run_progress(store, run_id).await?;
    store.remove_run(run_id).await?;
    Ok(RemovedRun {
        run_id,
        run_name: run.run_name,
    })
}

pub async fn remove_pending_task(
    store: &(impl crate::core::RunReadStore + RunTaskStore),
    run_id: i32,
    task_id: i64,
) -> Result<RemovedPendingTask, ApiError> {
    let _run = load_run_progress(store, run_id).await?;
    let removed = store.remove_pending_run_task(run_id, task_id).await?;
    if !removed {
        return Err(ApiError::BadRequest(format!(
            "run task {task_id} was not removed; only pending tasks can be removed"
        )));
    }
    Ok(RemovedPendingTask { run_id, task_id })
}

async fn preflight_task_batch(
    store: &(impl AggregationStore + RunTaskStore),
    run_id: i32,
    tasks: &[RunTaskInput],
) -> Result<(), ApiError> {
    let existing_tasks = if run_id > 0 {
        store.list_run_tasks(run_id).await?
    } else {
        Vec::new()
    };
    let context = build_task_preflight_context(&existing_tasks);
    validate_task_batch_against_context(tasks, context)
}

struct TaskPreflightContext {
    known_names: BTreeSet<String>,
    prior_sourceable_names: BTreeSet<String>,
    next_sequence: i32,
}

fn build_task_preflight_context(existing_tasks: &[RunTask]) -> TaskPreflightContext {
    let known_names = existing_tasks
        .iter()
        .map(|task| task.name.clone())
        .collect::<BTreeSet<_>>();
    let prior_sourceable_names = existing_tasks
        .iter()
        .filter(|task| task.task.is_sourceable())
        .map(|task| task.name.clone())
        .collect::<BTreeSet<_>>();
    let next_sequence = existing_tasks
        .iter()
        .map(|task| task.sequence_nr)
        .max()
        .unwrap_or(0)
        + 1;
    TaskPreflightContext {
        known_names,
        prior_sourceable_names,
        next_sequence,
    }
}

fn validate_task_batch_against_context(
    tasks: &[RunTaskInput],
    mut context: TaskPreflightContext,
) -> Result<(), ApiError> {
    for task in tasks {
        task.validate()
            .map_err(|err| ApiError::BadRequest(format!("invalid task entry: {err}")))?;

        for source_name in task.task.source_task_names() {
            if !context.prior_sourceable_names.contains(&source_name) {
                return Err(ApiError::BadRequest(format!(
                    "task source from_name='{}' does not reference a prior task in this run",
                    source_name
                )));
            }
        }

        let task_name = task
            .name
            .clone()
            .unwrap_or_else(|| crate::core::generated_task_name(&task.task, context.next_sequence));
        if !context.known_names.insert(task_name.clone()) {
            return Err(ApiError::BadRequest(format!(
                "task name '{}' is duplicated in this run",
                task_name
            )));
        }
        if task.task.is_sourceable() {
            context.prior_sourceable_names.insert(task_name);
        }
        context.next_sequence += 1;
    }
    Ok(())
}

fn format_clone_root_snapshot_name(
    source_run_name: &str,
    source_tasks: &[RunTask],
    snapshot: &RunStageSnapshot,
) -> String {
    match snapshot.task_id {
        None => format!(
            "clone_of:{}:root_snapshot:{}",
            source_run_name,
            snapshot.id.unwrap_or_default()
        ),
        Some(task_id) => {
            let task_name = source_tasks
                .iter()
                .find(|task| task.id == task_id)
                .map(|task| task.name.as_str())
                .unwrap_or("unknown_task");
            format!(
                "clone_of:{}:{}:snapshot:{}",
                source_run_name,
                task_name,
                snapshot.id.unwrap_or_default()
            )
        }
    }
}

async fn load_run_progress(
    store: &impl crate::core::RunReadStore,
    run_id: i32,
) -> Result<RunProgress, ApiError> {
    store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("run {run_id} not found")))
}

pub fn decode_integration_params(
    run_id: i32,
    value: serde_json::Value,
) -> Result<IntegrationParams, ApiError> {
    serde_json::from_value(value).map_err(|err| {
        ApiError::Internal(format!(
            "invalid integration_params for run {run_id}: {err}"
        ))
    })
}

fn read_default_run_add_toml() -> Result<toml::Value, ApiError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_RUN_CONFIG_PATH);
    read_toml_file(&path, "default run config")
}

fn read_toml_file(path: &Path, label: &str) -> Result<toml::Value, ApiError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        ApiError::Internal(format!("failed reading {label} {}: {err}", path.display()))
    })?;
    toml::from_str(&raw).map_err(|err| {
        ApiError::BadRequest(format!("failed parsing TOML {}: {err}", path.display()))
    })
}

fn parse_run_add_config_value(merged: toml::Value) -> Result<RunAddConfig, ApiError> {
    if merged
        .as_table()
        .and_then(|table| table.get("point_spec"))
        .is_some()
    {
        return Err(ApiError::BadRequest(
            "top-level [point_spec] is no longer supported; define dimensions in [evaluator]"
                .to_string(),
        ));
    }
    let parsed: RunAddConfig = merged
        .try_into()
        .map_err(|err| ApiError::BadRequest(format!("invalid run-add payload: {err}")))?;
    let name = parsed.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest(
            "invalid run name (`name`): expected non-empty string".to_string(),
        ));
    }
    Ok(RunAddConfig { name, ..parsed })
}

fn merge_toml(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, value) in overlay_table {
                if let Some(base_value) = base_table.get_mut(&key) {
                    merge_toml(base_value, value);
                } else {
                    base_table.insert(key, value);
                }
            }
        }
        (base_value, overlay_value) => {
            *base_value = overlay_value;
        }
    }
}
