use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand};
use gammaboard::core::{ControlPlaneStore, RunStatus};
use gammaboard::init_pg_store;
use gammaboard::preprocess::{RunAddConfig, preprocess_run_add};
use gammaboard::stores::RunReadStore;
use std::path::PathBuf;
use tracing::Instrument;

use super::shared::{RunSelection, RunStatusArg, init_cli_tracing};

const DEFAULT_RUN_CONFIG_PATH: &str = "configs/default.toml";

#[derive(Debug, Args)]
pub struct RunArgs {
    #[command(subcommand)]
    pub command: RunCommand,
}

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    Add {
        integration_params_file: PathBuf,
        #[arg(short = 's', long, value_enum, default_value_t = RunStatusArg::Pending)]
        status: RunStatusArg,
    },
    Start(RunSelection),
    Pause(RunSelection),
    Stop(RunSelection),
    Remove(RunSelection),
}

pub async fn run_run_commands(command: RunCommand, quiet: bool) -> Result<()> {
    let store = init_pg_store(10)
        .await
        .context("failed to initialize postgres store")?;
    init_cli_tracing(&store, quiet)?;
    let command_name = run_command_name(&command);
    let span = tracing::span!(
        tracing::Level::TRACE,
        "control_run_command",
        source = "control",
        command = command_name
    );

    async move {
        match command {
            RunCommand::Add {
                status,
                integration_params_file,
            } => {
                let config = load_run_add_config(&integration_params_file)?;
                let run_name = config.name.clone();
                tracing::info!(run = %run_name, "run-add preflight started");
                let processed = match preprocess_run_add(config) {
                    Ok(processed) => {
                        tracing::info!(run = %run_name, "run-add preflight finished");
                        processed
                    }
                    Err(err) => {
                        tracing::info!(run = %run_name, error = %err, "run-add preflight failed");
                        return Err(anyhow!("failed to preprocess run config: {err}"));
                    }
                };
                let point_spec = processed
                    .point_spec
                    .as_ref()
                    .ok_or_else(|| anyhow!("preprocessing did not resolve point_spec"))?;
                let run_status: RunStatus = status.into();
                let integration_params = serde_json::to_value(&processed.integration_params)
                    .map_err(|err| anyhow!("failed to serialize integration_params: {err}"))?;
                let run_id = store
                    .create_run(
                        run_status,
                        &processed.name,
                        &integration_params,
                        processed.target.as_ref(),
                        point_spec,
                        processed.evaluator_init_metadata.as_ref(),
                        processed.sampler_aggregator_init_metadata.as_ref(),
                    )
                    .await?;
                tracing::info!(
                    "created run_id={} name={} status={}",
                    run_id,
                    processed.name,
                    run_status.as_str()
                );
            }
            RunCommand::Start(selection) => {
                if selection.all {
                    let runs_updated = store.set_all_runs_status(RunStatus::Running).await?;
                    tracing::info!("started all runs: runs_updated={runs_updated}");
                } else {
                    for run_id in selection.run_ids {
                        store.set_run_status(run_id, RunStatus::Running).await?;
                        tracing::info!("run {run_id} started");
                    }
                }
            }
            RunCommand::Pause(selection) => {
                if selection.all {
                    let runs_updated = store.set_all_runs_status(RunStatus::Paused).await?;
                    let assignments_cleared = store.clear_all_desired_assignments().await?;
                    tracing::info!(
                        "paused all runs: runs_updated={} assignments_cleared={}",
                        runs_updated,
                        assignments_cleared
                    );
                } else {
                    for run_id in selection.run_ids {
                        store.set_run_status(run_id, RunStatus::Paused).await?;
                        let assignments_cleared =
                            store.clear_desired_assignments_for_run(run_id).await?;
                        tracing::info!(
                            "run {} paused assignments_cleared={}",
                            run_id,
                            assignments_cleared
                        );
                    }
                }
            }
            RunCommand::Stop(selection) => {
                if selection.all {
                    let runs_updated = store.set_all_runs_status(RunStatus::Cancelled).await?;
                    let assignments_cleared = store.clear_all_desired_assignments().await?;
                    tracing::info!(
                        "stopped all runs: runs_updated={} assignments_cleared={}",
                        runs_updated,
                        assignments_cleared
                    );
                } else {
                    for run_id in selection.run_ids {
                        store.set_run_status(run_id, RunStatus::Cancelled).await?;
                        let assignments_cleared =
                            store.clear_desired_assignments_for_run(run_id).await?;
                        tracing::info!(
                            "run {} stopped assignments_cleared={}",
                            run_id,
                            assignments_cleared
                        );
                    }
                }
            }
            RunCommand::Remove(selection) => {
                if selection.all {
                    let runs = store.get_all_runs().await?;
                    let mut removed = 0u64;
                    for run in runs {
                        store.remove_run(run.run_id).await?;
                        removed += 1;
                    }
                    tracing::info!("removed all runs: removed={removed}");
                } else {
                    for run_id in selection.run_ids {
                        store.remove_run(run_id).await?;
                        tracing::info!("removed run {run_id}");
                    }
                }
            }
        }
        Ok(())
    }
    .instrument(span)
    .await
}

fn run_command_name(command: &RunCommand) -> &'static str {
    match command {
        RunCommand::Add { .. } => "run_add",
        RunCommand::Start(_) => "run_start",
        RunCommand::Pause(_) => "run_pause",
        RunCommand::Stop(_) => "run_stop",
        RunCommand::Remove(_) => "run_remove",
    }
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

    Ok(RunAddConfig { name, ..parsed })
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
