use super::shared::{
    RunSelection, list_runs_by_name, resolve_run_ref, resolve_run_selection, with_control_store,
};
use anyhow::{Result, anyhow};
use clap::{Args, Subcommand};
use comfy_table::{Cell, CellAlignment, ContentArrangement, Table};
use gammaboard::PgStore;
use gammaboard::api::runs as run_api;
use gammaboard::config::CliConfig;
use gammaboard::core::{ControlPlaneStore, RunReadStore, RunTaskStore};
use std::path::PathBuf;

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
    let config = run_api::load_run_add_config_file(config_file).map_err(api_to_anyhow)?;
    let run_name = config.name.clone();
    tracing::info!(run = %run_name, "run-add preflight started");
    let created = run_api::create_run(store, config).await.map_err(|err| {
        tracing::info!(run = %run_name, error = %err, "run-add preflight failed");
        api_to_anyhow(err)
    })?;
    tracing::info!(run = %run_name, "run-add preflight finished");
    tracing::info!(
        "created run_id={} name={}",
        created.run_id,
        created.run_name
    );
    Ok(())
}

async fn clone_run(
    store: &PgStore,
    source_run_ref: &str,
    from_snapshot_id: i64,
    new_name: &str,
) -> Result<()> {
    let source_run = resolve_run_ref(store, source_run_ref).await?;
    let cloned = run_api::clone_run(store, source_run.run_id, from_snapshot_id, new_name)
        .await
        .map_err(api_to_anyhow)?;

    tracing::info!(
        run_id = cloned.run_id,
        new_name = cloned.run_name,
        source_run_id = cloned.source_run_id,
        from_snapshot_id = cloned.from_snapshot_id,
        cloned_tasks = cloned.cloned_tasks,
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
        let assignments_cleared = run_api::pause_run(store, run.run_id)
            .await
            .map_err(api_to_anyhow)?
            .assignments_cleared;
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
            let inserted = run_api::append_tasks(
                store,
                run_id,
                run_api::load_task_queue_file(&task_file).map_err(api_to_anyhow)?,
            )
            .await
            .map_err(api_to_anyhow)?;
            tracing::info!(
                run_id,
                tasks_added = inserted.tasks.len(),
                "appended run tasks"
            );
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
                    source_snapshot = format_task_snapshot_ref(task.task.source_snapshot_id()),
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

fn format_task_snapshot_ref(snapshot_id: Option<i64>) -> String {
    snapshot_id
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn format_task_snapshot_origin(snapshot_id: Option<i64>) -> String {
    snapshot_id
        .map(|value| value.to_string())
        .unwrap_or_default()
}

fn api_to_anyhow(err: gammaboard::api::ApiError) -> anyhow::Error {
    anyhow!(err.to_string())
}
