use super::shared::{
    RunSelection, list_runs_by_name, resolve_run_ref, resolve_run_selection, with_control_store,
};
use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand};
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};
use gammaboard::PgStore;
use gammaboard::config::CliConfig;
use gammaboard::core::{
    AggregationStore, ControlPlaneStore, IntegrationParams, RunReadStore, RunStageSnapshot,
    RunTask, RunTaskInputSpec, RunTaskSpec, RunTaskStore, StageSnapshotRef, resolve_task_queue,
};
use gammaboard::preprocess::{RunAddConfig, preflight_task_suffix, preprocess_run_add};
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
        from_snapshot_id: i64,
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
                    from_snapshot_id,
                    new_name,
                } => clone_run(&store, &source_run, from_snapshot_id, &new_name).await?,
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
    let initial_stage_snapshot = processed
        .initial_stage_snapshot
        .as_ref()
        .ok_or_else(|| anyhow!("preprocessing did not build initial stage snapshot"))?;
    preflight_task_batch(
        store,
        initial_stage_snapshot,
        processed
            .resolved_integration_params
            .as_ref()
            .ok_or_else(|| anyhow!("preprocessing did not resolve integration_params"))?,
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
    tracing::info!("created run_id={} name={}", run_id, processed.name);
    Ok(())
}

async fn clone_run(
    store: &PgStore,
    source_run_ref: &str,
    from_snapshot_id: i64,
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

    let snapshot = store.load_stage_snapshot(from_snapshot_id).await?;
    let snapshot =
        snapshot.ok_or_else(|| anyhow!("stage snapshot {from_snapshot_id} not found"))?;
    if snapshot.run_id != source_run.run_id {
        return Err(anyhow!(
            "stage snapshot {} belongs to run {}, not source run {}",
            from_snapshot_id,
            snapshot.run_id,
            source_run.run_id
        ));
    }

    let source_tasks = store.list_run_tasks(source_run.run_id).await?;
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

    tracing::info!(
        run_id,
        new_name,
        source_run_id = source_run.run_id,
        from_snapshot_id,
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
            let run_spec = load_run_spec_for_preflight(store, run_id).await?;
            let base_snapshot = load_append_base_snapshot(store, run_id).await?;
            preflight_task_batch(
                store,
                &base_snapshot,
                &run_spec,
                &tasks,
                &run.point_spec
                    .ok_or_else(|| anyhow!("run {run_id} is missing point_spec"))?,
            )
            .await?;
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
                    spawned_from = format_task_snapshot_origin(task.spawned_from_snapshot_id),
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
    from_snapshot: &gammaboard::core::RunStageSnapshot,
) -> Result<Vec<RunTaskSpec>> {
    let source_index = match from_snapshot.task_id {
        Some(task_id) => Some(
            source_tasks
                .iter()
                .position(|task| task.id == task_id)
                .ok_or_else(|| anyhow!("run task {task_id} not found"))?,
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
                snapshot_id: from_snapshot
                    .id
                    .ok_or_else(|| anyhow!("source stage snapshot is missing id"))?,
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
    store: &(impl AggregationStore + RunReadStore),
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
    let _run = store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| anyhow!("run {run_id} not found"))?;
    let base_snapshot = load_append_base_snapshot(store, run_id).await?;
    let base_sampler_aggregator = base_snapshot.sampler_aggregator;
    let base_materializer = base_snapshot.materializer.config;
    let base_batch_transforms = base_snapshot.batch_transforms;
    resolve_task_queue(
        &base_sampler_aggregator,
        &base_materializer,
        &base_batch_transforms,
        &parsed.task_queue,
    )
    .map_err(|err| anyhow!("invalid task_queue entry: {err}"))
}

async fn load_append_base_snapshot(
    store: &impl AggregationStore,
    run_id: i32,
) -> Result<RunStageSnapshot> {
    store
        .load_latest_stage_snapshot_before_sequence(run_id, i32::MAX)
        .await?
        .ok_or_else(|| anyhow!("run {run_id} has no base stage snapshot"))
}

async fn load_run_spec_for_preflight(
    store: &impl RunReadStore,
    run_id: i32,
) -> Result<IntegrationParams> {
    let run = store
        .get_run_progress(run_id)
        .await?
        .ok_or_else(|| anyhow!("run {run_id} not found"))?;
    serde_json::from_value(
        run.integration_params
            .ok_or_else(|| anyhow!("run {run_id} is missing integration_params"))?,
    )
    .map_err(|err| anyhow!("invalid integration_params for run {run_id}: {err}"))
}

async fn preflight_task_batch(
    store: &impl AggregationStore,
    base_snapshot: &RunStageSnapshot,
    integration_params: &IntegrationParams,
    tasks: &[RunTaskSpec],
    point_spec: &gammaboard::evaluation::PointSpec,
) -> Result<()> {
    let mut referenced_snapshots = std::collections::BTreeMap::new();
    for task in tasks {
        if let Some(start_from) = task.start_from() {
            let snapshot = store
                .load_stage_snapshot(start_from.snapshot_id)
                .await?
                .ok_or_else(|| {
                    anyhow!(
                        "task start_from references snapshot {} but no stage snapshot exists",
                        start_from.snapshot_id
                    )
                })?;
            referenced_snapshots.insert(start_from.snapshot_id, snapshot);
        }
    }
    let mut evaluator = integration_params.evaluator.build()?;
    preflight_task_suffix(
        base_snapshot,
        &referenced_snapshots,
        tasks,
        &mut *evaluator,
        point_spec,
    )
    .map_err(|err| anyhow!("failed to preflight task batch: {err}"))
}

fn format_task_snapshot_ref(start_from: Option<&gammaboard::core::StageSnapshotRef>) -> String {
    start_from
        .map(|snapshot| snapshot.snapshot_id.to_string())
        .unwrap_or_default()
}

fn format_task_snapshot_origin(snapshot_id: Option<i64>) -> String {
    snapshot_id
        .map(|value| value.to_string())
        .unwrap_or_default()
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
