use super::shared::{
    RunSelection, list_runs_by_name, resolve_run_ref, resolve_run_selection, with_control_store,
};
use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand};
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};
use gammaboard::PgStore;
use gammaboard::config::CliConfig;
use gammaboard::core::{
    ControlPlaneStore, IntegrationParams, RunReadStore, RunTask, RunTaskInputSpec, RunTaskSpec,
    RunTaskStore, TaskSnapshotRef, resolve_task_queue,
};
use gammaboard::preprocess::{RunAddConfig, preprocess_run_add};
use serde::Deserialize;
use std::path::PathBuf;

const DEFAULT_RUN_CONFIG_PATH: &str = "configs/default.toml";

#[derive(Debug, Args)]
pub struct RunArgs {
    #[command(subcommand)]
    pub command: RunCommand,
}

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    Add {
        config_file: PathBuf,
    },
    Clone {
        source_run: String,
        from_task_id: i64,
        new_name: String,
    },
    List {
        run_name: Option<String>,
    },
    Pause(RunSelection),
    Remove(RunSelection),
    Task(TaskArgs),
}

#[derive(Debug, Args)]
pub struct TaskArgs {
    #[command(subcommand)]
    command: TaskCommand,
}

#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    Add { run: String, task_file: PathBuf },
    List { run: String },
    Remove { run: String, task_id: i64 },
}

pub async fn run_run_commands(command: RunCommand, config: &CliConfig, quiet: bool) -> Result<()> {
    with_control_store(
        config,
        10,
        quiet,
        run_command_name(&command),
        |store| async move {
            match command {
                RunCommand::Add { config_file } => run_add(&store, &config_file).await?,
                RunCommand::Clone {
                    source_run,
                    from_task_id,
                    new_name,
                } => clone_run(&store, &source_run, from_task_id, &new_name).await?,
                RunCommand::List { run_name } => list_runs(&store, run_name.as_deref()).await?,
                RunCommand::Pause(selection) => pause_runs(&store, selection).await?,
                RunCommand::Remove(selection) => remove_runs(&store, selection).await?,
                RunCommand::Task(args) => run_task_command(&store, args.command).await?,
            }
            Ok(())
        },
    )
    .await
}

fn run_command_name(command: &RunCommand) -> &'static str {
    match command {
        RunCommand::Add { .. } => "run_add",
        RunCommand::Clone { .. } => "run_clone",
        RunCommand::List { .. } => "run_list",
        RunCommand::Pause(_) => "run_pause",
        RunCommand::Remove(_) => "run_remove",
        RunCommand::Task(_) => "run_task",
    }
}

async fn run_add(store: &PgStore, config_file: &PathBuf) -> Result<()> {
    let config = load_run_add_config(config_file)?;
    let run_name = config.name.clone();
    tracing::info!(run = %run_name, "run-add preflight started");
    let processed = preprocess_run_add(config).map_err(|err| {
        tracing::info!(run = %run_name, error = %err, "run-add preflight failed");
        anyhow!("failed to preprocess run config: {err}")
    })?;
    tracing::info!(run = %run_name, "run-add preflight finished");

    let point_spec = processed
        .point_spec
        .as_ref()
        .ok_or_else(|| anyhow!("preprocessing did not resolve point_spec"))?;
    let integration_params = serde_json::to_value(
        processed
            .resolved_integration_params
            .as_ref()
            .ok_or_else(|| anyhow!("preprocessing did not resolve integration_params"))?,
    )
    .map_err(|err| anyhow!("failed to serialize integration_params: {err}"))?;
    let initial_tasks = processed.resolved_task_queue.clone().unwrap_or_default();
    validate_task_snapshot_refs(store, &initial_tasks).await?;
    let run_id = store
        .create_run(
            &processed.name,
            &integration_params,
            processed.target.as_ref(),
            point_spec,
            processed.evaluator_init_metadata.as_ref(),
            processed.sampler_aggregator_init_metadata.as_ref(),
            &initial_tasks,
        )
        .await?;
    tracing::info!("created run_id={} name={}", run_id, processed.name);
    Ok(())
}

async fn clone_run(
    store: &PgStore,
    source_run_ref: &str,
    from_task_id: i64,
    new_name: &str,
) -> Result<()> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err(anyhow!(
            "invalid run name (`new_name`): expected non-empty string"
        ));
    }

    let source_run = resolve_run_ref(store, source_run_ref).await?;
    let point_spec = source_run
        .point_spec
        .clone()
        .ok_or_else(|| anyhow!("source run {} is missing point_spec", source_run.run_id))?;
    let integration_params = source_run.integration_params.clone().ok_or_else(|| {
        anyhow!(
            "source run {} is missing integration_params",
            source_run.run_id
        )
    })?;

    let source_tasks = store.list_run_tasks(source_run.run_id).await?;
    let cloned_tasks = clone_task_suffix(&source_tasks, source_run.run_id, from_task_id)?;

    let snapshot = store
        .get_latest_task_stage_snapshot(source_run.run_id, from_task_id)
        .await?;
    if snapshot.is_none() {
        return Err(anyhow!(
            "cannot clone from run {} task {}: no stage snapshot exists",
            source_run.run_id,
            from_task_id
        ));
    }

    let run_id = store
        .create_run(
            new_name,
            &integration_params,
            source_run.target.as_ref(),
            &point_spec,
            source_run.evaluator_init_metadata.as_ref(),
            source_run.sampler_aggregator_init_metadata.as_ref(),
            &cloned_tasks,
        )
        .await?;

    tracing::info!(
        run_id,
        new_name,
        source_run_id = source_run.run_id,
        from_task_id,
        cloned_tasks = cloned_tasks.len(),
        "cloned run"
    );
    Ok(())
}

async fn list_runs(store: &PgStore, run_name: Option<&str>) -> Result<()> {
    let runs = match run_name {
        Some(run_name) => list_runs_by_name(store, run_name).await?,
        None => store.get_all_runs().await?,
    };
    print_run_table(runs);
    Ok(())
}

async fn pause_runs(store: &PgStore, selection: RunSelection) -> Result<()> {
    if selection.all {
        let assignments_cleared = store.clear_all_desired_assignments().await?;
        tracing::info!("paused all runs: assignments_cleared={assignments_cleared}");
        return Ok(());
    }

    for run in resolve_run_selection(store, selection).await? {
        let assignments_cleared = store.clear_desired_assignments_for_run(run.run_id).await?;
        tracing::info!(
            "run {} ({}) paused assignments_cleared={}",
            run.run_id,
            run.run_name,
            assignments_cleared
        );
    }
    Ok(())
}

async fn remove_runs(store: &PgStore, selection: RunSelection) -> Result<()> {
    if selection.all {
        let runs = store.get_all_runs().await?;
        let mut removed = 0u64;
        for run in runs {
            store.remove_run(run.run_id).await?;
            removed += 1;
        }
        tracing::info!("removed all runs: removed={removed}");
        return Ok(());
    }

    for run in resolve_run_selection(store, selection).await? {
        store.remove_run(run.run_id).await?;
        tracing::info!("removed run {} ({})", run.run_id, run.run_name);
    }
    Ok(())
}

async fn run_task_command(store: &PgStore, command: TaskCommand) -> Result<()> {
    match command {
        TaskCommand::Add { run, task_file } => {
            let run = resolve_run_ref(store, &run).await?;
            let run_id = run.run_id;
            let tasks = resolve_task_queue_file_for_run(store, run_id, &task_file).await?;
            validate_task_snapshot_refs(store, &tasks).await?;
            let inserted = store.append_run_tasks(run_id, &tasks).await?;
            tracing::info!(run_id, tasks_added = inserted.len(), "appended run tasks");
        }
        TaskCommand::List { run } => {
            let run = resolve_run_ref(store, &run).await?;
            let run_id = run.run_id;
            let tasks = store.list_run_tasks(run_id).await?;
            for task in tasks {
                tracing::info!(
                    run_id = task.run_id,
                    task_id = task.id,
                    sequence_nr = task.sequence_nr,
                    state = task.state.as_str(),
                    kind = task.task.kind_str(),
                    nr_produced_samples = task.nr_produced_samples,
                    nr_completed_samples = task.nr_completed_samples,
                    start_from = format_task_snapshot_ref(task.task.start_from()),
                    spawned_from = format_task_snapshot_origin(
                        task.spawned_from_run_id,
                        task.spawned_from_task_id
                    ),
                    failure_reason = task.failure_reason.as_deref().unwrap_or(""),
                    "run task"
                );
            }
        }
        TaskCommand::Remove { run, task_id } => {
            let run = resolve_run_ref(store, &run).await?;
            let run_id = run.run_id;
            let removed = store.remove_pending_run_task(run_id, task_id).await?;
            if !removed {
                return Err(anyhow!(
                    "run task {task_id} was not removed; only pending tasks can be removed"
                ));
            }
            tracing::info!(run_id, task_id, "removed pending run task");
        }
    }
    Ok(())
}

fn print_run_table(runs: Vec<gammaboard::stores::RunProgress>) {
    if runs.is_empty() {
        println!("no runs found");
        return;
    }

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("ID").set_alignment(CellAlignment::Center),
        Cell::new("Name").set_alignment(CellAlignment::Center),
        Cell::new("State").set_alignment(CellAlignment::Center),
        Cell::new("Produced").set_alignment(CellAlignment::Center),
        Cell::new("Completed").set_alignment(CellAlignment::Center),
    ]);

    for run in runs {
        table.add_row(vec![
            run.run_id.to_string(),
            run.run_name,
            run.lifecycle_state,
            run.nr_produced_samples.to_string(),
            run.nr_completed_samples.to_string(),
        ]);
    }

    println!("{table}");
}

fn clone_task_suffix(
    source_tasks: &[RunTask],
    source_run_id: i32,
    from_task_id: i64,
) -> Result<Vec<RunTaskSpec>> {
    let source_index = source_tasks
        .iter()
        .position(|task| task.id == from_task_id)
        .ok_or_else(|| anyhow!("run task {from_task_id} not found"))?;

    let mut cloned_tasks = source_tasks
        .iter()
        .skip(source_index + 1)
        .map(|task| task.task.clone())
        .collect::<Vec<_>>();

    if let Some(first_executable) = cloned_tasks
        .iter_mut()
        .find(|task| !matches!(task, RunTaskSpec::Pause))
    {
        set_task_start_from(
            first_executable,
            TaskSnapshotRef {
                run_id: source_run_id,
                task_id: from_task_id,
            },
        );
    }

    Ok(cloned_tasks)
}

fn set_task_start_from(task: &mut RunTaskSpec, start_from: TaskSnapshotRef) {
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

#[derive(Debug, Deserialize)]
struct TaskQueueFile {
    task_queue: Vec<RunTaskInputSpec>,
}

fn load_run_add_config(path: &PathBuf) -> Result<RunAddConfig> {
    let default_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_RUN_CONFIG_PATH);
    let mut merged = read_run_add_toml(&default_path)?;
    let overlay = read_run_add_toml(path)?;
    merge_toml(&mut merged, overlay);
    if merged
        .as_table()
        .and_then(|table| table.get("point_spec"))
        .is_some()
    {
        return Err(anyhow!(
            "top-level [point_spec] is no longer supported; define dimensions in [evaluator]"
        ));
    }
    let parsed: RunAddConfig = merged
        .try_into()
        .map_err(|err| anyhow!("invalid run-add payload: {err}"))?;
    let name = parsed.name.trim().to_string();
    if name.is_empty() {
        return Err(anyhow!(
            "invalid run name (`name`): expected non-empty string"
        ));
    }
    if let Some(task_queue) = parsed.task_queue.as_ref() {
        if task_queue.is_empty() {
            return Err(anyhow!(
                "invalid task_queue: expected at least one task when set"
            ));
        }
        for task in task_queue {
            task.validate()
                .map_err(|err| anyhow!("invalid task_queue entry: {err}"))?;
        }
    }

    Ok(RunAddConfig { name, ..parsed })
}

async fn resolve_task_queue_file_for_run(
    store: &(impl RunTaskStore + RunReadStore),
    run_id: i32,
    path: &PathBuf,
) -> Result<Vec<RunTaskSpec>> {
    let parsed: TaskQueueFile = read_run_add_toml(path)?
        .try_into()
        .map_err(|err| anyhow!("invalid run-task payload: {err}"))?;
    if parsed.task_queue.is_empty() {
        return Err(anyhow!("invalid task_queue: expected at least one task"));
    }
    for task in &parsed.task_queue {
        task.validate()
            .map_err(|err| anyhow!("invalid task_queue entry: {err}"))?;
    }
    let run = store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| anyhow!("run {run_id} not found"))?;
    let integration_params: IntegrationParams = serde_json::from_value(
        run.integration_params
            .ok_or_else(|| anyhow!("run {run_id} is missing integration_params"))?,
    )
    .map_err(|err| anyhow!("invalid integration_params for run {run_id}: {err}"))?;
    let existing_tasks = store.list_run_tasks(run_id).await?;
    let mut base_sampler_aggregator = integration_params.sampler_aggregator;
    let mut base_parametrization = integration_params.parametrization;
    for task in existing_tasks {
        if let RunTaskSpec::Sample {
            sampler_aggregator,
            parametrization,
            ..
        } = task.task
        {
            base_sampler_aggregator = sampler_aggregator;
            base_parametrization = parametrization;
        }
    }
    resolve_task_queue(
        &base_sampler_aggregator,
        &base_parametrization,
        &parsed.task_queue,
    )
    .map_err(|err| anyhow!("invalid task_queue entry: {err}"))
}

async fn validate_task_snapshot_refs(
    store: &impl RunReadStore,
    tasks: &[RunTaskSpec],
) -> Result<()> {
    for task in tasks {
        if let Some(start_from) = task.start_from() {
            let snapshot = store
                .get_latest_task_stage_snapshot(start_from.run_id, start_from.task_id)
                .await?;
            if snapshot.is_none() {
                return Err(anyhow!(
                    "task start_from references run {} task {} but no stage snapshot exists",
                    start_from.run_id,
                    start_from.task_id
                ));
            }
        }
    }
    Ok(())
}

fn format_task_snapshot_ref(start_from: Option<&gammaboard::core::TaskSnapshotRef>) -> String {
    start_from
        .map(|snapshot| format!("{}:{}", snapshot.run_id, snapshot.task_id))
        .unwrap_or_default()
}

fn format_task_snapshot_origin(run_id: Option<i32>, task_id: Option<i64>) -> String {
    match (run_id, task_id) {
        (Some(run_id), Some(task_id)) => format!("{run_id}:{task_id}"),
        _ => String::new(),
    }
}

fn read_run_add_toml(path: &PathBuf) -> Result<toml::Value> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed reading run-add TOML from {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("failed parsing TOML {}", path.display()))
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
