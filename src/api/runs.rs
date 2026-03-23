use crate::api::ApiError;
use crate::core::{
    AggregationStore, ControlPlaneStore, IntegrationParams, RunStageSnapshot, RunTask,
    RunTaskInputSpec, RunTaskSpec, RunTaskStore, StageSnapshotRef, resolve_task_queue,
};
use crate::evaluation::PointSpec;
use crate::preprocess::{RunAddConfig, preflight_task_suffix, preprocess_run_add};
use crate::stores::RunProgress;
use serde::Deserialize;
use std::collections::BTreeMap;
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

#[derive(Debug, Clone, Deserialize)]
pub struct TaskQueueFile {
    #[serde(default)]
    pub task: Option<RunTaskInputSpec>,
    #[serde(default)]
    pub task_queue: Option<Vec<RunTaskInputSpec>>,
}

impl TaskQueueFile {
    pub fn into_tasks(self) -> Vec<RunTaskInputSpec> {
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
    let parsed: TaskQueueFile = toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("invalid run-task payload: {err}")))?;
    validate_task_inputs(&parsed)?;
    Ok(parsed)
}

pub fn load_task_queue_file(path: &Path) -> Result<TaskQueueFile, ApiError> {
    let parsed: TaskQueueFile = read_toml_file(path, "run-task TOML")?
        .try_into()
        .map_err(|err| ApiError::BadRequest(format!("invalid run-task payload: {err}")))?;
    validate_task_inputs(&parsed)?;
    Ok(parsed)
}

pub async fn create_run(
    store: &(impl ControlPlaneStore + AggregationStore),
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

    preflight_task_batch(
        store,
        initial_stage_snapshot,
        resolved_integration_params,
        &initial_tasks,
        point_spec,
    )
    .await?;

    let run_id = store
        .create_run(
            &processed.name,
            &integration_params,
            processed.target.as_ref(),
            point_spec,
            processed.evaluator_init_metadata.as_ref(),
            processed.sampler_aggregator_init_metadata.as_ref(),
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
    let cloned_tasks = clone_task_suffix(&source_tasks, &snapshot)?;
    let run_id = store
        .create_run(
            new_name,
            &integration_params,
            source_run.target.as_ref(),
            &point_spec,
            source_run.evaluator_init_metadata.as_ref(),
            source_run.sampler_aggregator_init_metadata.as_ref(),
            &RunStageSnapshot {
                id: None,
                run_id: 0,
                task_id: None,
                sequence_nr: None,
                queue_empty: snapshot.queue_empty,
                sampler_snapshot: snapshot.sampler_snapshot.clone(),
                observable_state: snapshot.observable_state.clone(),
                sampler_aggregator: snapshot.sampler_aggregator.clone(),
                materializer: snapshot.materializer.clone(),
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
    let tasks = resolve_task_file_for_run(store, run_id, task_file).await?;
    let run = load_run_progress(store, run_id).await?;
    let point_spec = run
        .point_spec
        .as_ref()
        .ok_or_else(|| ApiError::Internal(format!("run {run_id} is missing point_spec")))?;
    let integration_params = decode_integration_params(
        run_id,
        run.integration_params.ok_or_else(|| {
            ApiError::Internal(format!("run {run_id} is missing integration_params"))
        })?,
    )?;
    let base_snapshot = load_append_base_snapshot(store, run_id).await?;
    preflight_task_batch(
        store,
        &base_snapshot,
        &integration_params,
        &tasks,
        point_spec,
    )
    .await?;
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

async fn resolve_task_file_for_run(
    store: &(impl AggregationStore + crate::core::RunReadStore),
    run_id: i32,
    task_file: TaskQueueFile,
) -> Result<Vec<RunTaskSpec>, ApiError> {
    let _run = load_run_progress(store, run_id).await?;
    let base_snapshot = load_append_base_snapshot(store, run_id).await?;
    let tasks = task_file.into_tasks();
    resolve_task_queue(
        &base_snapshot.sampler_aggregator,
        &base_snapshot.materializer.config,
        &base_snapshot.batch_transforms,
        &tasks,
    )
    .map_err(ApiError::BadRequest)
}

async fn load_append_base_snapshot(
    store: &impl AggregationStore,
    run_id: i32,
) -> Result<RunStageSnapshot, ApiError> {
    store
        .load_latest_stage_snapshot_before_sequence(run_id, i32::MAX)
        .await?
        .ok_or_else(|| ApiError::Internal(format!("run {run_id} has no base stage snapshot")))
}

async fn preflight_task_batch(
    store: &impl AggregationStore,
    base_snapshot: &RunStageSnapshot,
    integration_params: &IntegrationParams,
    tasks: &[RunTaskSpec],
    point_spec: &PointSpec,
) -> Result<(), ApiError> {
    let mut referenced_snapshots = BTreeMap::new();
    for task in tasks {
        if let Some(start_from) = task.start_from() {
            let snapshot = store
                .load_stage_snapshot(start_from.snapshot_id)
                .await?
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "task start_from references snapshot {} but no stage snapshot exists",
                        start_from.snapshot_id
                    ))
                })?;
            referenced_snapshots.insert(start_from.snapshot_id, snapshot);
        }
    }
    let mut evaluator = integration_params
        .evaluator
        .build()
        .map_err(|err| ApiError::BadRequest(format!("failed to build evaluator: {err}")))?;
    preflight_task_suffix(
        base_snapshot,
        &referenced_snapshots,
        tasks,
        &mut *evaluator,
        point_spec,
    )
    .map_err(|err| ApiError::BadRequest(format!("failed to preflight task batch: {err}")))
}

fn clone_task_suffix(
    source_tasks: &[RunTask],
    from_snapshot: &RunStageSnapshot,
) -> Result<Vec<RunTaskSpec>, ApiError> {
    let source_index = match from_snapshot.task_id {
        Some(task_id) => Some(
            source_tasks
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| ApiError::NotFound(format!("run task {task_id} not found")))?,
        ),
        None => None,
    };
    let mut cloned_tasks = source_tasks
        .iter()
        .skip(source_index.map_or(0, |index| index + 1))
        .map(|task| task.task.clone())
        .collect::<Vec<_>>();
    if let Some(first_executable) = cloned_tasks
        .iter_mut()
        .find(|task| !matches!(task, RunTaskSpec::Pause))
    {
        set_task_start_from(
            first_executable,
            StageSnapshotRef {
                snapshot_id: from_snapshot.id.ok_or_else(|| {
                    ApiError::Internal("source stage snapshot is missing id".to_string())
                })?,
            },
        );
    }
    Ok(cloned_tasks)
}

fn set_task_start_from(task: &mut RunTaskSpec, start_from: StageSnapshotRef) {
    match task {
        RunTaskSpec::Sample {
            start_from: task_start_from,
            ..
        }
        | RunTaskSpec::Image {
            start_from: task_start_from,
            ..
        }
        | RunTaskSpec::PlotLine {
            start_from: task_start_from,
            ..
        } => {
            *task_start_from = Some(start_from);
        }
        RunTaskSpec::Pause => {}
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
    if let Some(task_queue) = parsed.task_queue.as_ref() {
        for task in task_queue {
            task.validate()
                .map_err(|err| ApiError::BadRequest(format!("invalid task_queue entry: {err}")))?;
        }
    }
    Ok(RunAddConfig { name, ..parsed })
}

fn validate_task_inputs(task_file: &TaskQueueFile) -> Result<(), ApiError> {
    for task in task_file
        .task
        .iter()
        .chain(task_file.task_queue.as_deref().unwrap_or(&[]).iter())
    {
        task.validate()
            .map_err(|err| ApiError::BadRequest(format!("invalid task entry: {err}")))?;
    }
    Ok(())
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
