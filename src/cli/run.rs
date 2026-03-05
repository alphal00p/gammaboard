use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand};
use gammaboard::batch::PointSpec;
use gammaboard::core::{ControlPlaneStore, RunStatus};
use gammaboard::init_pg_store;
use gammaboard::stores::RunReadStore;
use serde_json::Value as JsonValue;
use std::{env, fs, path::PathBuf};
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

pub async fn run_run_commands(command: RunCommand) -> Result<()> {
    let store = init_pg_store(10)
        .await
        .context("failed to initialize postgres store")?;
    init_cli_tracing(&store)?;
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
                let (name, integration_params, target, point_spec) =
                    parse_run_add_payload(read_merged_run_add_toml(&integration_params_file)?)?;
                let run_status: RunStatus = status.into();
                let run_id = store
                    .create_run(
                        run_status,
                        &name,
                        &integration_params,
                        target.as_ref(),
                        &point_spec,
                    )
                    .await?;
                println!(
                    "created run_id={} name={} status={}",
                    run_id,
                    name,
                    run_status.as_str()
                );
            }
            RunCommand::Start(selection) => {
                if selection.all {
                    let runs_updated = store.set_all_runs_status(RunStatus::Running).await?;
                    println!("started all runs: runs_updated={runs_updated}");
                } else {
                    for run_id in selection.run_ids {
                        store.set_run_status(run_id, RunStatus::Running).await?;
                        println!("run {run_id} started");
                    }
                }
            }
            RunCommand::Pause(selection) => {
                if selection.all {
                    let runs_updated = store.set_all_runs_status(RunStatus::Paused).await?;
                    let assignments_cleared = store.clear_all_desired_assignments().await?;
                    println!(
                        "paused all runs: runs_updated={} assignments_cleared={}",
                        runs_updated, assignments_cleared
                    );
                } else {
                    for run_id in selection.run_ids {
                        store.set_run_status(run_id, RunStatus::Paused).await?;
                        let assignments_cleared =
                            store.clear_desired_assignments_for_run(run_id).await?;
                        println!(
                            "run {} paused assignments_cleared={}",
                            run_id, assignments_cleared
                        );
                    }
                }
            }
            RunCommand::Stop(selection) => {
                if selection.all {
                    let runs_updated = store.set_all_runs_status(RunStatus::Cancelled).await?;
                    let assignments_cleared = store.clear_all_desired_assignments().await?;
                    println!(
                        "stopped all runs: runs_updated={} assignments_cleared={}",
                        runs_updated, assignments_cleared
                    );
                } else {
                    for run_id in selection.run_ids {
                        store.set_run_status(run_id, RunStatus::Cancelled).await?;
                        let assignments_cleared =
                            store.clear_desired_assignments_for_run(run_id).await?;
                        println!(
                            "run {} stopped assignments_cleared={}",
                            run_id, assignments_cleared
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
                    println!("removed all runs: removed={removed}");
                } else {
                    for run_id in selection.run_ids {
                        store.remove_run(run_id).await?;
                        println!("removed run {run_id}");
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

fn read_run_add_toml(path: &PathBuf) -> Result<JsonValue> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed reading run-add TOML from {}", path.display()))?;
    let toml_value: toml::Value =
        toml::from_str(&raw).with_context(|| format!("failed parsing TOML {}", path.display()))?;
    let json_value = serde_json::to_value(toml_value)
        .with_context(|| format!("failed converting TOML to JSON {}", path.display()))?;
    if !json_value.is_object() {
        return Err(anyhow!("run-add TOML must be a table"));
    }
    Ok(json_value)
}

fn merge_json(base: &mut JsonValue, overlay: JsonValue) {
    match (base, overlay) {
        (JsonValue::Object(base_obj), JsonValue::Object(overlay_obj)) => {
            for (key, value) in overlay_obj {
                if let Some(base_value) = base_obj.get_mut(&key) {
                    merge_json(base_value, value);
                } else {
                    base_obj.insert(key, value);
                }
            }
        }
        (base_value, overlay_value) => {
            *base_value = overlay_value;
        }
    }
}

fn read_merged_run_add_toml(path: &PathBuf) -> Result<JsonValue> {
    let default_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_RUN_CONFIG_PATH);
    let mut merged = read_run_add_toml(&default_path)?;
    let overlay = read_run_add_toml(path)?;
    merge_json(&mut merged, overlay);
    Ok(merged)
}

fn parse_run_add_payload(
    raw: JsonValue,
) -> Result<(String, JsonValue, Option<JsonValue>, PointSpec)> {
    let mut root = raw
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow!("run-add payload must be a table"))?;

    let name_value = root
        .remove("name")
        .ok_or_else(|| anyhow!("missing run name (`name`) in run-add payload"))?;
    let name = name_value
        .as_str()
        .map(|value| value.trim().to_string())
        .ok_or_else(|| anyhow!("invalid run name (`name`): expected non-empty string"))?;
    if name.is_empty() {
        return Err(anyhow!(
            "invalid run name (`name`): expected non-empty string"
        ));
    }

    let point_spec_value = root
        .remove("point_spec")
        .ok_or_else(|| anyhow!("missing [point_spec] section in run-add payload"))?;
    let point_spec: PointSpec = serde_json::from_value(point_spec_value)
        .map_err(|err| anyhow!("invalid point_spec: {err}"))?;

    let target = root.remove("target");

    Ok((name, JsonValue::Object(root), target, point_spec))
}
