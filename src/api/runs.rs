use crate::api::{ApiError, toml_template};
use crate::core::IntegrationParams;
use crate::core::{
    AccumulatorConfig, AggregationStore, ControlPlaneStore, EvaluatorConfig, RunStageSnapshot,
    RunTask, RunTaskInput, RunTaskState, RunTaskStore, SamplerQueueTuning, SourceRefSpec,
};
use crate::preprocess::{RunAddConfig, preprocess_run_add};
use crate::stores::RunProgress;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const DEFAULT_RUN_CONFIG_TOML: &str = include_str!("../config_defaults/run.toml");

#[derive(Debug, Clone)]
pub struct CreatedRun {
    pub run_id: i32,
    pub run_name: String,
    pub tasks_created: usize,
}

#[derive(Debug, Clone)]
pub struct ChildRunRequest {
    pub parent_run_id: i32,
    pub parent_task_id: Option<i64>,
    pub spawn_kind: String,
    pub spawn_label: Option<String>,
    pub run_toml: String,
    pub replacements: BTreeMap<String, toml::Value>,
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
    let overlay_has_evaluator = toml_has_key(&overlay, "evaluator");
    merge_toml(&mut merged, overlay);
    if !overlay_has_evaluator {
        remove_toml_key(&mut merged, "evaluator");
    }
    let expanded = toml_template::expand_toml_template(merged)?;
    let mut config = parse_run_add_config_value(expanded.value)?;
    config.original_toml = Some(raw.to_string());
    Ok(config)
}

pub fn parse_run_add_config_toml_with_replacements(
    raw: &str,
    replacements: BTreeMap<String, toml::Value>,
) -> Result<RunAddConfig, ApiError> {
    let mut merged = read_default_run_add_toml()?;
    let overlay = toml::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("failed parsing run TOML: {err}")))?;
    let overlay_has_evaluator = toml_has_key(&overlay, "evaluator");
    merge_toml(&mut merged, overlay);
    if !overlay_has_evaluator {
        remove_toml_key(&mut merged, "evaluator");
    }
    toml_template::merge_replacements(&mut merged, replacements)?;
    let expanded = toml_template::expand_toml_template(merged)?;
    let mut config = parse_run_add_config_value(expanded.value)?;
    config.original_toml = Some(raw.to_string());
    Ok(config)
}

/// Loads a run-add TOML file and merges it over the built-in default run template.
pub fn load_run_add_config_file(path: &Path) -> Result<RunAddConfig, ApiError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        ApiError::Internal(format!(
            "failed reading run-add TOML {}: {err}",
            path.display()
        ))
    })?;
    let mut merged = read_default_run_add_toml()?;
    let overlay = toml::from_str(&raw).map_err(|err| {
        ApiError::BadRequest(format!("failed parsing TOML {}: {err}", path.display()))
    })?;
    let overlay_has_evaluator = toml_has_key(&overlay, "evaluator");
    merge_toml(&mut merged, overlay);
    if !overlay_has_evaluator {
        remove_toml_key(&mut merged, "evaluator");
    }
    let expanded = toml_template::expand_toml_template(merged)?;
    let mut config = parse_run_add_config_value(expanded.value)?;
    config.original_toml = Some(raw);
    Ok(config)
}

/// Parses task-append TOML supporting `task`, `task_queue`, or both.
pub fn parse_task_queue_toml(raw: &str) -> Result<TaskQueueFile, ApiError> {
    toml_template::parse_templated_toml(raw, "run-task payload")
}

/// Loads and parses a task file from disk.
pub fn load_task_queue_file(path: &Path) -> Result<TaskQueueFile, ApiError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        ApiError::Internal(format!(
            "failed reading run-task TOML {}: {err}",
            path.display()
        ))
    })?;
    parse_task_queue_toml(&raw)
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
    let mut initial_stage_snapshot = processed
        .initial_stage_snapshot
        .as_ref()
        .cloned()
        .ok_or_else(|| {
            ApiError::Internal("preprocessing did not build initial stage snapshot".to_string())
        })?;
    initial_stage_snapshot.evaluator = resolved_integration_params.evaluator.clone();

    preflight_task_batch(
        store,
        initial_stage_snapshot.run_id,
        &initial_tasks,
        resolved_integration_params.evaluator.clone(),
    )
    .await?;

    let run_toml = stored_run_toml(
        &processed,
        resolved_integration_params,
        processed.target.as_ref(),
        &initial_tasks,
    )?;

    let run_id = store
        .create_run(
            &processed.name,
            &run_toml,
            &integration_params,
            processed.target.as_ref(),
            domain,
            &initial_stage_snapshot,
            &initial_tasks,
        )
        .await?;

    Ok(CreatedRun {
        run_id,
        run_name: processed.name,
        tasks_created: initial_tasks.len(),
    })
}

pub async fn create_child_run(
    store: &(impl ControlPlaneStore + AggregationStore + RunTaskStore),
    request: ChildRunRequest,
) -> Result<CreatedRun, ApiError> {
    let spawn_kind = request.spawn_kind.trim();
    if spawn_kind.is_empty() {
        return Err(ApiError::BadRequest(
            "child run spawn_kind must be non-empty".to_string(),
        ));
    }
    let config =
        parse_run_add_config_toml_with_replacements(&request.run_toml, request.replacements)?;
    let created = create_run(store, config).await?;
    store
        .set_run_parent_metadata(
            created.run_id,
            request.parent_run_id,
            request.parent_task_id,
            spawn_kind,
            request.spawn_label.as_deref(),
        )
        .await?;
    Ok(created)
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
                evaluator: snapshot.evaluator.clone(),
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
    preflight_task_batch(store, run_id, &tasks, integration_params.evaluator).await?;
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
    evaluator: Option<EvaluatorConfig>,
) -> Result<(), ApiError> {
    let existing_tasks = if run_id > 0 {
        store.list_run_tasks(run_id).await?
    } else {
        Vec::new()
    };
    let mut context = TaskPreflightContext::from_existing_tasks(&existing_tasks, evaluator)?;
    context.validate_batch(tasks)
}

struct TaskPreflightContext {
    known_names: BTreeSet<String>,
    prior_sourceable_names: BTreeSet<String>,
    effective_accumulator_by_name: BTreeMap<String, AccumulatorConfig>,
    effective_evaluator_by_name: BTreeMap<String, EvaluatorConfig>,
    current_accumulator: Option<AccumulatorConfig>,
    current_evaluator: Option<EvaluatorConfig>,
    next_sequence: i32,
    root_evaluator: Option<EvaluatorConfig>,
}

impl TaskPreflightContext {
    fn from_existing_tasks(
        existing_tasks: &[RunTask],
        evaluator: Option<EvaluatorConfig>,
    ) -> Result<Self, ApiError> {
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
        let mut context = Self {
            known_names,
            prior_sourceable_names,
            effective_accumulator_by_name: BTreeMap::new(),
            effective_evaluator_by_name: BTreeMap::new(),
            current_accumulator: None,
            current_evaluator: evaluator.clone(),
            next_sequence,
            root_evaluator: evaluator,
        };
        let mut ordered_tasks = existing_tasks.iter().collect::<Vec<_>>();
        ordered_tasks.sort_by_key(|task| (task.sequence_nr, task.id));
        for task in ordered_tasks {
            context.record_existing_task(task)?;
        }
        Ok(context)
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
        let effective_accumulator = self.resolve_effective_accumulator(task)?;
        let effective_evaluator = if task.task.runs_in_control_plane() {
            None
        } else {
            Some(self.resolve_effective_evaluator(task)?)
        };
        if let Some(effective_evaluator) = effective_evaluator.as_ref() {
            self.validate_evaluator_domain(effective_evaluator)?;
        }
        if let Some(config) = effective_accumulator.as_ref() {
            if task.task.runs_in_control_plane() {
                if let Some(effective_evaluator) = self.current_evaluator.as_ref() {
                    self.validate_accumulator_against_evaluator(effective_evaluator, config)?;
                }
            } else {
                let Some(effective_evaluator) = effective_evaluator.as_ref() else {
                    return Err(ApiError::BadRequest(format!(
                        "task '{}' defines an accumulator but has no effective evaluator configuration",
                        task_name
                    )));
                };
                self.validate_accumulator_against_evaluator(effective_evaluator, config)?;
            }
        }
        if task.task.is_sourceable() {
            self.prior_sourceable_names.insert(task_name.clone());
        }
        if let Some(effective_evaluator) = effective_evaluator {
            self.effective_evaluator_by_name
                .insert(task_name.clone(), effective_evaluator.clone());
            self.current_evaluator = Some(effective_evaluator);
        }
        if let Some(config) = effective_accumulator {
            self.effective_accumulator_by_name
                .insert(task_name, config.clone());
            self.current_accumulator = Some(config);
        }
        self.next_sequence += 1;
        Ok(())
    }

    fn record_existing_task(&mut self, task: &RunTask) -> Result<(), ApiError> {
        let input = RunTaskInput {
            name: Some(task.name.clone()),
            task: task.task.clone(),
        };
        if !input.task.runs_in_control_plane() {
            let effective_evaluator = self.resolve_effective_evaluator(&input)?;
            self.effective_evaluator_by_name
                .insert(task.name.clone(), effective_evaluator.clone());
            self.current_evaluator = Some(effective_evaluator);
        }
        let effective_accumulator = self.resolve_effective_accumulator(&input)?;
        if let Some(config) = effective_accumulator {
            self.effective_accumulator_by_name
                .insert(task.name.clone(), config.clone());
            self.current_accumulator = Some(config);
        }
        Ok(())
    }

    fn resolve_effective_evaluator(
        &self,
        task: &RunTaskInput,
    ) -> Result<EvaluatorConfig, ApiError> {
        if let Some(config) = task.task.evaluator_config() {
            return Ok(config);
        }

        match task.task.evaluator_source() {
            Some(SourceRefSpec::Latest) | None => self.current_evaluator.clone().ok_or_else(|| {
                ApiError::BadRequest(
                    "task has no effective evaluator configuration; set one explicitly first"
                        .to_string(),
                )
            }),
            Some(SourceRefSpec::FromName(source_name)) => self
                .effective_evaluator_by_name
                .get(&source_name)
                .cloned()
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "task references evaluator source task '{}' but no effective evaluator is available from it",
                        source_name
                    ))
                }),
        }
    }

    fn resolve_effective_accumulator(
        &self,
        task: &RunTaskInput,
    ) -> Result<Option<AccumulatorConfig>, ApiError> {
        if let Some(config) = task
            .task
            .new_accumulator_config()
            .map_err(|err| ApiError::BadRequest(format!("invalid task entry: {err}")))?
        {
            return Ok(Some(config));
        }

        match &task.task {
            crate::core::RunTaskSpec::Sample { .. } => match task.task.sample_accumulator_source() {
                Some(SourceRefSpec::Latest) => self.current_accumulator.clone().ok_or_else(|| {
                    ApiError::BadRequest(
                        "sample task has no effective accumulator configuration; set one explicitly first or add a prior accumulator-producing task".to_string(),
                    )
                }).map(Some),
                Some(SourceRefSpec::FromName(source_name)) => self
                    .effective_accumulator_by_name
                    .get(&source_name)
                    .cloned()
                    .ok_or_else(|| {
                        ApiError::BadRequest(format!(
                            "sample task references accumulator source task '{}' but no effective accumulator is available from it",
                            source_name
                        ))
                    })
                    .map(Some),
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn validate_accumulator_against_evaluator(
        &self,
        evaluator: &EvaluatorConfig,
        config: &AccumulatorConfig,
    ) -> Result<(), ApiError> {
        evaluator
            .validate_accumulator_config(config)
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
        Ok(())
    }

    fn validate_evaluator_domain(&self, evaluator: &EvaluatorConfig) -> Result<(), ApiError> {
        let Some(root_evaluator) = self.root_evaluator.as_ref() else {
            return Ok(());
        };
        let root_domain = root_evaluator
            .resolve_domain()
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
        let evaluator_domain = evaluator
            .resolve_domain()
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
        if evaluator_domain != root_domain {
            return Err(ApiError::BadRequest(format!(
                "task evaluator domain {:?} does not match run domain {:?}",
                evaluator_domain, root_domain
            )));
        }
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
    toml::from_str(DEFAULT_RUN_CONFIG_TOML).map_err(|err| {
        ApiError::Internal(format!("failed parsing embedded default run config: {err}"))
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

fn stored_run_toml(
    processed: &RunAddConfig,
    integration_params: &IntegrationParams,
    target: Option<&serde_json::Value>,
    task_queue: &[RunTaskInput],
) -> Result<String, ApiError> {
    if let Some(original_toml) = processed.original_toml.as_ref() {
        return Ok(original_toml.clone());
    }
    canonical_run_toml(&processed.name, integration_params, target, task_queue)
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

fn toml_has_key(value: &toml::Value, key: &str) -> bool {
    value
        .as_table()
        .is_some_and(|table| table.contains_key(key))
}

fn remove_toml_key(value: &mut toml::Value, key: &str) {
    if let Some(table) = value.as_table_mut() {
        table.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{
        RunTaskSpec, SampleStopCondition, SamplerAggregatorConfig, SamplerAggregatorSourceSpec,
    };
    use crate::evaluation::UnitEvaluatorParams;
    use crate::sampling::NaiveMonteCarloSamplerParams;

    fn scalar_unit_evaluator() -> EvaluatorConfig {
        EvaluatorConfig::Unit {
            params: UnitEvaluatorParams::default(),
        }
    }

    #[test]
    fn parse_run_add_expands_typed_top_level_replacements() {
        let raw = r#"
replacements = { run_name = "templated-run", samples = 12, enabled_failures = [1, 2] }

name = '$(run_name:"fallback-run")'

[evaluator]
kind = "unit"
continuous_dims = "$(continuous_dims:1)"
discrete_dims = 0
fail_on_batch_nrs = "$(enabled_failures:[])"

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = "$(samples:8)" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#;
        let config = parse_run_add_config_toml(raw).expect("templated run config");

        assert_eq!(config.name, "templated-run");
        assert_eq!(config.original_toml.as_deref(), Some(raw));
        let Some(EvaluatorConfig::Unit { params }) = config.integration_params.evaluator else {
            panic!("expected unit evaluator");
        };
        assert_eq!(params.continuous_dims, 1);
        assert_eq!(params.fail_on_batch_nrs, vec![1, 2]);
        let Some(tasks) = config.task_queue else {
            panic!("missing task queue");
        };
        let RunTaskSpec::Sample { stop_condition, .. } = &tasks[0].task else {
            panic!("expected sample task");
        };
        assert_eq!(stop_condition.max_samples, Some(12));
    }

    #[test]
    fn parse_run_add_allows_external_replacement_injection() {
        let mut replacements = BTreeMap::new();
        replacements.insert(
            "run_name".to_string(),
            toml::Value::String("child".to_string()),
        );
        replacements.insert("samples".to_string(), toml::Value::Integer(32));
        let config = parse_run_add_config_toml_with_replacements(
            r#"
name = '$(run_name:"fallback")'

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = "$(samples:8)" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
            replacements,
        )
        .expect("run config");

        assert_eq!(config.name, "child");
        let tasks = config.task_queue.expect("tasks");
        let RunTaskSpec::Sample { stop_condition, .. } = &tasks[0].task else {
            panic!("expected sample task");
        };
        assert_eq!(stop_condition.max_samples, Some(32));
    }

    #[test]
    fn stored_run_toml_preserves_original_submitted_text() {
        let raw = r#"

replacements = { run_name = "stored-template", samples = 12 }

name = '$(run_name:"fallback-run")'

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = "$(samples:8)" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#;
        let processed = preprocess_run_add(parse_run_add_config_toml(raw).expect("run config"))
            .expect("preprocess");
        let integration_params = processed
            .resolved_integration_params
            .as_ref()
            .expect("resolved integration params");
        let tasks = processed.resolved_task_queue.clone().unwrap_or_default();

        let stored = stored_run_toml(
            &processed,
            integration_params,
            processed.target.as_ref(),
            &tasks,
        )
        .expect("stored run toml");

        assert_eq!(stored, raw);
    }

    #[test]
    fn parse_task_queue_expands_typed_top_level_replacements() {
        let tasks = parse_task_queue_toml(
            r#"
replacements = { task_name = "templated-sample", samples = 16 }

[task]
name = '$(task_name:"fallback-sample")'
kind = "sample"
stop_condition = { max_samples = "$(samples:8)" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
        )
        .expect("templated task config")
        .into_tasks();

        assert_eq!(tasks[0].name.as_deref(), Some("templated-sample"));
        let RunTaskSpec::Sample { stop_condition, .. } = &tasks[0].task else {
            panic!("expected sample task");
        };
        assert_eq!(stop_condition.max_samples, Some(16));
    }

    #[test]
    fn parse_run_add_accepts_task_level_evaluator_sources() {
        let config = parse_run_add_config_toml(
            r#"
name = "task-evaluators"

[evaluator]
kind = "symbolica"
expr = "1"
args = ["x"]

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"
accumulator = "scalar"

[[task_queue]]
name = "sample-a"
kind = "sample"
stop_condition = { max_samples = 8 }
evaluator = { config = { kind = "symbolica", expr = "x", args = ["x"] } }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
accumulator = "latest"

[[task_queue]]
name = "sample-b"
kind = "sample"
stop_condition = { max_samples = 8 }
evaluator = { from_name = "sample-a" }
sampler_aggregator = "latest"
accumulator = "latest"
"#,
        )
        .expect("task-level evaluator config");

        assert_eq!(config.task_queue.expect("tasks").len(), 3);
    }

    #[test]
    fn parse_run_add_accepts_parameter_scan_task() {
        let config = parse_run_add_config_toml(
            r#"
name = "scan-parent"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "scan"
kind = "parameter_scan"
max_concurrent_runs = 2
trial_run_toml = """
name = "child-$(scale:1)"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
name = "sample"
kind = "sample"
stop_condition = { max_samples = 4 }
measurement = { quantity = "central_value" }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"""

[task_queue.parameter]
name = "scale"
linspace = { start = 1.0, stop = 3.0, count = 3 }

[task_queue.measurement]
source_task = "sample"
"#,
        )
        .expect("run config");

        let RunTaskSpec::ParameterScan {
            parameter,
            parameters,
            max_concurrent_runs,
            ..
        } = &config.task_queue.expect("tasks")[0].task
        else {
            panic!("expected parameter scan task");
        };
        let parameter = parameter
            .as_ref()
            .or_else(|| parameters.first())
            .expect("parameter");
        assert_eq!(parameter.name, "scale");
        assert_eq!(
            parameter.values().expect("values"),
            vec![
                toml::Value::Float(1.0),
                toml::Value::Float(2.0),
                toml::Value::Float(3.0),
            ]
        );
        assert_eq!(*max_concurrent_runs, 2);
    }

    #[test]
    fn parse_run_add_accepts_multi_parameter_scan_task() {
        let config = parse_run_add_config_toml(
            r#"
name = "scan-parent"

[[task_queue]]
name = "scan"
kind = "parameter_scan"
max_concurrent_runs = 2
trial_run_toml = "name = \"child\"\n"

[[task_queue.parameters]]
name = "scale"
values = [1, 2]

[[task_queue.parameters]]
name = "offset"
values = [0.0, 1.0]

[task_queue.measurement]
source_task = "sample"
"#,
        )
        .expect("run config");

        let RunTaskSpec::ParameterScan {
            parameter,
            parameters,
            ..
        } = &config.task_queue.expect("tasks")[0].task
        else {
            panic!("expected parameter scan task");
        };
        assert!(parameter.is_none());
        assert_eq!(parameters.len(), 2);
        assert_eq!(parameters[0].name, "scale");
        assert_eq!(parameters[1].name, "offset");
    }

    #[test]
    fn parse_run_add_rejects_task_level_evaluator_domain_changes() {
        let config = parse_run_add_config_toml(
            r#"
name = "bad-task-evaluator-domain"

[evaluator]
kind = "symbolica"
expr = "1"
args = ["x"]

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 8 }
evaluator = { config = { kind = "symbolica", expr = "x + y", args = ["x", "y"] } }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
accumulator = { config = "scalar" }
"#,
        )
        .expect("run config");
        let mut context = TaskPreflightContext::from_existing_tasks(
            &[],
            config.integration_params.evaluator.clone(),
        )
        .expect("context");
        let err = context
            .validate_batch(config.task_queue.as_ref().expect("tasks"))
            .expect_err("domain-changing evaluator should fail");

        assert!(err.to_string().contains("does not match run domain"));
    }

    #[test]
    fn bundled_run_and_task_templates_parse_after_replacement_expansion() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for relative_path in [
            "resources/templates/runs/ghost_bump.toml",
            "resources/templates/runs/gammaloop-6photons-hsigma-ecom-scan.toml",
            "resources/templates/runs/hyperparameter-tuning-symbolica.toml",
            "resources/templates/runs/parameter-scan-symbolica.toml",
            "resources/templates/runs/symbolica-havana-pdf-1d2d.toml",
            "resources/templates/runs/symbolica-variable-smooth-compiled.toml",
            "resources/templates/runs/symbolica-variable-smooth-egobox.toml",
            "resources/templates/tasks/pdf_adaptation_image.toml",
            "resources/templates/tasks/sample_monte_carlo_real.toml",
            "resources/templates/tasks/train_sample.toml",
            "process_api/examples/python_gammaloop_observable/run.toml",
            "process_api/examples/python_sampler_symbolica_havana/run.toml",
            "process_api/examples/python_scalar_sin/run.toml",
            "process_api/examples/rust_breit_wigner_evaluator/run.toml",
            "ops/ubelix/resources/templates/runs/ghost_bump_madnis.toml",
            "ops/ubelix/resources/templates/tasks/train_sample.toml",
        ] {
            let raw = fs::read_to_string(root.join(relative_path)).expect("template file");
            if relative_path.contains("/tasks/") {
                parse_task_queue_toml(&raw)
                    .unwrap_or_else(|err| panic!("{relative_path} should parse: {err}"));
            } else {
                parse_run_add_config_toml(&raw)
                    .unwrap_or_else(|err| panic!("{relative_path} should parse: {err}"));
            }
        }

        if cfg!(feature = "gammaloop") {
            for relative_path in [
                "resources/templates/runs/gammaloop.toml",
                "ops/ubelix/resources/templates/runs/epem_a_tth.toml",
            ] {
                let raw = fs::read_to_string(root.join(relative_path)).expect("template file");
                parse_run_add_config_toml(&raw)
                    .unwrap_or_else(|err| panic!("{relative_path} should parse: {err}"));
            }
        }
    }

    #[test]
    fn parse_run_add_accepts_accumulator_discrete_projections() {
        let config = parse_run_add_config_toml(
            r#"
name = "histogram-config"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 2

[[task_queue]]
name = "accumulator"
kind = "set_accumulator"

[task_queue.accumulator]
kind = "scalar"

[task_queue.accumulator.discrete_projections]
normalization = "conditional_mean"

[[task_queue.accumulator.discrete_projections.items]]
name = "channel_for_spin_0"
dims = [1]
fixed_dims = { "0" = 0 }
"#,
        )
        .expect("run config");

        let RunTaskSpec::SetAccumulator { accumulator } = &config.task_queue.unwrap()[0].task
        else {
            panic!("expected set_accumulator");
        };
        let projections = accumulator
            .discrete_projections()
            .expect("projection config");
        assert_eq!(
            projections.normalization,
            crate::core::DiscreteProjectionNormalization::ConditionalMean
        );
    }

    #[test]
    fn parse_run_add_accepts_unit_evaluator_multiple_fail_batches() {
        let config = parse_run_add_config_toml(
            r#"
name = "unit-fail-batches"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0
fail_on_batch_nrs = [1, 2, 3]

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 8 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo" } }
"#,
        )
        .expect("run config");

        let Some(EvaluatorConfig::Unit { params }) = config.integration_params.evaluator else {
            panic!("expected unit evaluator");
        };
        assert_eq!(params.fail_on_batch_nrs, vec![1, 2, 3]);
    }

    #[test]
    fn parse_run_add_accepts_naive_monte_carlo_materializer_failure() {
        let config = parse_run_add_config_toml(
            r#"
name = "materializer-failure"

[evaluator]
kind = "unit"
continuous_dims = 1
discrete_dims = 0

[[task_queue]]
kind = "sample"
stop_condition = { max_samples = 8 }
accumulator = { config = "scalar" }
sampler_aggregator = { config = { kind = "naive_monte_carlo", fail_on_materialize_batch_nr = 1 } }
"#,
        )
        .expect("run config");

        let Some(tasks) = config.task_queue else {
            panic!("missing task queue");
        };
        let RunTaskSpec::Sample {
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config { config }),
            ..
        } = &tasks[0].task
        else {
            panic!("expected sample task with sampler config");
        };
        let SamplerAggregatorConfig::NaiveMonteCarlo { params, .. } = config else {
            panic!("expected naive monte carlo config");
        };
        assert_eq!(params.fail_on_materialize_batch_nr, Some(1));
    }

    fn sample_task(accumulator: Option<crate::core::AccumulatorSourceSpec>) -> RunTaskInput {
        RunTaskInput {
            name: None,
            task: RunTaskSpec::Sample {
                stop_condition: SampleStopCondition {
                    max_samples: Some(10),
                    ..SampleStopCondition::default()
                },
                measurement: None,
                evaluator: None,
                sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config {
                    config: SamplerAggregatorConfig::NaiveMonteCarlo {
                        params: NaiveMonteCarloSamplerParams::default(),
                        materializer: None,
                    },
                }),
                accumulator,
                queue_tuning: None,
                batch_transforms: None,
            },
        }
    }

    #[test]
    fn preflight_rejects_first_sample_without_accumulator_state() {
        let mut context =
            TaskPreflightContext::from_existing_tasks(&[], Some(scalar_unit_evaluator()))
                .expect("context");
        let error = context
            .validate_batch(&[sample_task(None)])
            .expect_err("missing accumulator should fail");
        assert!(
            error
                .to_string()
                .contains("sample task has no effective accumulator configuration")
        );
    }

    #[test]
    fn parse_run_add_allows_controller_only_run_without_root_evaluator() {
        let config = parse_run_add_config_toml(
            r#"
name = "controller-only"

[[task_queue]]
name = "scan"
kind = "parameter_scan"
trial_run_toml = "name = \"child\"\n[evaluator]\nkind = \"unit\"\ncontinuous_dims = 1\ndiscrete_dims = 0\n"

[task_queue.parameter]
name = "scale"
values = [1]

[task_queue.measurement]
source_task = "sample"
"#,
        )
        .expect("controller-only run config");
        assert!(config.integration_params.evaluator.is_none());
        preprocess_run_add(config).expect("controller-only preprocessing");
    }

    #[test]
    fn preflight_rejects_compute_task_without_effective_evaluator() {
        let mut context = TaskPreflightContext::from_existing_tasks(&[], None).expect("context");
        let error = context
            .validate_batch(&[RunTaskInput {
                name: Some("sample".to_string()),
                task: RunTaskSpec::Sample {
                    stop_condition: SampleStopCondition {
                        max_samples: Some(10),
                        ..SampleStopCondition::default()
                    },
                    measurement: None,
                    evaluator: None,
                    sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config {
                        config: SamplerAggregatorConfig::NaiveMonteCarlo {
                            params: NaiveMonteCarloSamplerParams::default(),
                            materializer: None,
                        },
                    }),
                    accumulator: Some(crate::core::AccumulatorSourceSpec::Config {
                        config: AccumulatorConfig::scalar(),
                    }),
                    queue_tuning: None,
                    batch_transforms: None,
                },
            }])
            .expect_err("missing evaluator should fail");
        assert!(
            error
                .to_string()
                .contains("task has no effective evaluator configuration")
        );
    }

    #[test]
    fn preflight_accepts_sample_after_set_accumulator() {
        let mut context =
            TaskPreflightContext::from_existing_tasks(&[], Some(scalar_unit_evaluator()))
                .expect("context");
        context
            .validate_batch(&[
                RunTaskInput {
                    name: Some("prep".to_string()),
                    task: RunTaskSpec::SetAccumulator {
                        accumulator: AccumulatorConfig::scalar(),
                    },
                },
                sample_task(None),
            ])
            .expect("set_accumulator should establish accumulator state");
    }

    #[test]
    fn preflight_rejects_incompatible_gammaloop_accumulator_for_scalar_evaluator() {
        let mut context =
            TaskPreflightContext::from_existing_tasks(&[], Some(scalar_unit_evaluator()))
                .expect("context");
        let error = context
            .validate_batch(&[RunTaskInput {
                name: Some("prep".to_string()),
                task: RunTaskSpec::SetAccumulator {
                    accumulator: AccumulatorConfig::Gammaloop,
                },
            }])
            .expect_err("gammaloop accumulator should fail for scalar unit evaluator");
        assert!(
            error
                .to_string()
                .contains("does not support accumulator config \"gammaloop\"")
        );
    }
}
