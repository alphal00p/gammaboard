use crate::api::ApiError;
use crate::core::IntegrationParams;
use crate::core::{
    AggregationStore, ControlPlaneStore, RunStageSnapshot, RunTask, RunTaskInput, RunTaskState,
    RunTaskStore, SamplerQueueTuning,
};
use crate::preprocess::{RunAddConfig, preprocess_run_add};
use crate::stores::RunProgress;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const DEFAULT_RUN_CONFIG_PATH: &str = "configs/runs/default.toml";

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

#[derive(Debug, Clone)]
pub struct UpdatedTaskQueueTuning {
    pub run_id: i32,
    pub task: RunTask,
}

#[derive(Debug, Clone, Serialize)]
struct RunReproToml {
    name: String,
    #[serde(flatten)]
    integration_params: IntegrationParams,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    task_queue: Vec<RunTaskInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskQueueFile {
    #[serde(default)]
    pub task: Option<RunTaskInput>,
    #[serde(default)]
    pub task_queue: Option<Vec<RunTaskInput>>,
}

impl TaskQueueFile {
    /// Normalizes accepted task-file shapes into a single ordered task list.
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

/// Parses run-add TOML, merging it over the default run config.
pub fn parse_run_add_config_toml(raw: &str) -> Result<RunAddConfig, ApiError> {
    let mut merged = read_default_run_add_toml()?;
    let overlay = toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("failed parsing run TOML: {err}")))?;
    merge_toml(&mut merged, overlay);
    parse_run_add_config_value(merged)
}

/// Loads a run-add TOML file and merges it over `configs/runs/default.toml`.
pub fn load_run_add_config_file(path: &Path) -> Result<RunAddConfig, ApiError> {
    let default_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_RUN_CONFIG_PATH);
    let mut merged = read_toml_file(&default_path, "default run config")?;
    let overlay = read_toml_file(path, "run-add TOML")?;
    merge_toml(&mut merged, overlay);
    parse_run_add_config_value(merged)
}

/// Parses task-append TOML supporting `task`, `task_queue`, or both.
pub fn parse_task_queue_toml(raw: &str) -> Result<TaskQueueFile, ApiError> {
    toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("invalid run-task payload: {err}")))
}

/// Loads and parses a task file from disk.
pub fn load_task_queue_file(path: &Path) -> Result<TaskQueueFile, ApiError> {
    read_toml_file(path, "run-task TOML")?
        .try_into()
        .map_err(|err| ApiError::BadRequest(format!("invalid run-task payload: {err}")))
}

/// Creates a run, persists the root stage snapshot, and appends initial tasks if provided.
pub async fn create_run(
    store: &(impl ControlPlaneStore + AggregationStore + RunTaskStore),
    config: RunAddConfig,
) -> Result<CreatedRun, ApiError> {
    let processed = preprocess_run_add(config)?;
    let domain = processed
        .domain
        .as_ref()
        .ok_or_else(|| ApiError::Internal("preprocessing did not resolve domain".to_string()))?;
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
            &canonical_run_toml(
                &processed.name,
                resolved_integration_params,
                processed.target.as_ref(),
                &initial_tasks,
            )?,
            &integration_params,
            processed.target.as_ref(),
            domain,
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

/// Clones a run from a specific persisted stage snapshot into a new idle run.
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
    let domain = source_run.domain.clone().ok_or_else(|| {
        ApiError::Internal(format!("source run {source_run_id} is missing domain"))
    })?;
    let integration_params = source_run.integration_params.clone().ok_or_else(|| {
        ApiError::Internal(format!(
            "source run {source_run_id} is missing integration_params"
        ))
    })?;
    let integration_params_typed: IntegrationParams =
        serde_json::from_value(integration_params.clone()).map_err(|err| {
            ApiError::Internal(format!(
                "source run {source_run_id} has invalid integration_params payload: {err}"
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
            &canonical_run_toml(
                new_name,
                &integration_params_typed,
                source_run.target.as_ref(),
                &cloned_tasks,
            )?,
            &integration_params,
            source_run.target.as_ref(),
            &domain,
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

fn canonical_run_toml(
    name: &str,
    integration_params: &IntegrationParams,
    target: Option<&serde_json::Value>,
    task_queue: &[RunTaskInput],
) -> Result<String, ApiError> {
    toml::to_string(&RunReproToml {
        name: name.to_string(),
        integration_params: integration_params.clone(),
        target: target.cloned().filter(|value| !value.is_null()),
        task_queue: task_queue.to_vec(),
    })
    .map_err(|err| ApiError::Internal(format!("failed to serialize run TOML: {err}")))
}

/// Appends validated tasks to an existing run.
pub async fn append_tasks(
    store: &(impl AggregationStore + crate::core::RunReadStore + RunTaskStore),
    run_id: i32,
    task_file: TaskQueueFile,
) -> Result<AppendedTasks, ApiError> {
    let tasks = task_file.into_tasks();
    let _run = load_run_progress(store, run_id).await?;
    preflight_task_batch(store, run_id, &tasks).await?;
    let tasks = store.append_run_tasks(run_id, &tasks).await?;
    Ok(AppendedTasks { tasks })
}

/// Pauses a run by clearing desired node assignments for that run.
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

/// Removes a run and its dependent data.
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

/// Removes a pending task from a run.
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

pub async fn update_task_queue_tuning(
    store: &(impl crate::core::RunReadStore + RunTaskStore),
    run_id: i32,
    task_id: i64,
    queue_tuning: Option<SamplerQueueTuning>,
) -> Result<UpdatedTaskQueueTuning, ApiError> {
    let _run = load_run_progress(store, run_id).await?;
    if let Some(queue_tuning) = queue_tuning.as_ref() {
        queue_tuning
            .validate()
            .map_err(|err| ApiError::BadRequest(format!("invalid queue_tuning: {err}")))?;
    }
    let task = store
        .update_run_task_queue_tuning(run_id, task_id, queue_tuning)
        .await?;
    Ok(UpdatedTaskQueueTuning { run_id, task })
}

pub async fn export_run_repro_toml(
    store: &(impl crate::core::RunReadStore + RunTaskStore),
    run_id: i32,
) -> Result<String, ApiError> {
    let run = load_run_progress(store, run_id).await?;
    let integration_params_value = run
        .integration_params
        .clone()
        .ok_or_else(|| ApiError::Internal(format!("run {run_id} is missing integration_params")))?;
    let integration_params: IntegrationParams = serde_json::from_value(integration_params_value)
        .map_err(|err| {
            ApiError::Internal(format!(
                "run {run_id} has invalid integration_params payload: {err}"
            ))
        })?;
    let completed_tasks = store
        .list_run_tasks(run_id)
        .await?
        .into_iter()
        .filter(|task| matches!(task.state, RunTaskState::Completed))
        .map(|task| RunTaskInput {
            name: Some(task.name),
            task: task.task,
        })
        .collect::<Vec<_>>();

    toml::to_string(&RunReproToml {
        name: run.run_name,
        integration_params,
        target: run.target.filter(|value| !value.is_null()),
        task_queue: completed_tasks,
    })
    .map_err(|err| ApiError::Internal(format!("failed to serialize run repro TOML: {err}")))
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
    let mut context = TaskPreflightContext::from_existing_tasks(&existing_tasks);
    context.validate_batch(tasks)
}

struct TaskPreflightContext {
    known_names: BTreeSet<String>,
    prior_sourceable_names: BTreeSet<String>,
    next_sequence: i32,
}

impl TaskPreflightContext {
    fn from_existing_tasks(existing_tasks: &[RunTask]) -> Self {
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
        Self {
            known_names,
            prior_sourceable_names,
            next_sequence,
        }
    }

    fn validate_batch(&mut self, tasks: &[RunTaskInput]) -> Result<(), ApiError> {
        for task in tasks {
            self.validate_task(task)?;
        }
        Ok(())
    }

    fn validate_task(&mut self, task: &RunTaskInput) -> Result<(), ApiError> {
        task.validate()
            .map_err(|err| ApiError::BadRequest(format!("invalid task entry: {err}")))?;
        for source_name in task.task.source_task_names() {
            if !self.prior_sourceable_names.contains(&source_name) {
                return Err(ApiError::BadRequest(format!(
                    "task source from_name='{}' does not reference a prior task in this run",
                    source_name
                )));
            }
        }
        let task_name = task
            .name
            .clone()
            .unwrap_or_else(|| crate::core::generated_task_name(&task.task, self.next_sequence));
        if !self.known_names.insert(task_name.clone()) {
            return Err(ApiError::BadRequest(format!(
                "task name '{}' is duplicated in this run",
                task_name
            )));
        }
        if task.task.is_sourceable() {
            self.prior_sourceable_names.insert(task_name);
        }
        self.next_sequence += 1;
        Ok(())
    }
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
        .and_then(|table| table.get("point_spec").or_else(|| table.get("domain")))
        .is_some()
    {
        return Err(ApiError::BadRequest(
            "top-level [point_spec] or [domain] is no longer supported; define layout in [evaluator]"
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
