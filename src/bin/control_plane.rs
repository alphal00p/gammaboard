use clap::{Args, Parser, Subcommand, ValueEnum};
use gammaboard::batch::PointSpec;
use gammaboard::core::{ControlPlaneStore, DesiredAssignment, RunStatus, WorkerRole};
use gammaboard::stores::RunReadStore;
use gammaboard::{BinResult, init_pg_store};
use serde_json::Value as JsonValue;
use std::{fs, path::PathBuf};

const DEFAULT_RUN_CONFIG_PATH: &str = "configs/default.toml";

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RoleArg {
    Evaluator,
    SamplerAggregator,
}

impl From<RoleArg> for WorkerRole {
    fn from(value: RoleArg) -> Self {
        match value {
            RoleArg::Evaluator => WorkerRole::Evaluator,
            RoleArg::SamplerAggregator => WorkerRole::SamplerAggregator,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RunStatusArg {
    Pending,
    WarmUp,
    Running,
    Completed,
    Paused,
    Cancelled,
}

impl From<RunStatusArg> for RunStatus {
    fn from(value: RunStatusArg) -> Self {
        match value {
            RunStatusArg::Pending => RunStatus::Pending,
            RunStatusArg::WarmUp => RunStatus::WarmUp,
            RunStatusArg::Running => RunStatus::Running,
            RunStatusArg::Completed => RunStatus::Completed,
            RunStatusArg::Paused => RunStatus::Paused,
            RunStatusArg::Cancelled => RunStatus::Cancelled,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "control_plane")]
#[command(about = "Control-plane admin CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Assign {
        node_id: String,
        role: RoleArg,
        run_id: i32,
    },
    Unassign {
        node_id: String,
        role: RoleArg,
    },
    ListAssignments {
        node_id: Option<String>,
    },
    RunAdd {
        integration_params_file: PathBuf,
        #[arg(short = 's', long, value_enum, default_value_t = RunStatusArg::Pending)]
        status: RunStatusArg,
    },
    RunStart(RunSelection),
    RunPause(RunSelection),
    RunStop(RunSelection),
    RunRemove(RunSelection),
    NodeStop(NodeSelection),
}

#[derive(Debug, Args)]
struct RunSelection {
    #[arg(short = 'a', long = "all", conflicts_with = "run_ids")]
    all: bool,
    #[arg(value_name = "RUN_ID", required_unless_present = "all")]
    run_ids: Vec<i32>,
}

#[derive(Debug, Args)]
struct NodeSelection {
    #[arg(short = 'a', long = "all", conflicts_with = "node_ids")]
    all: bool,
    #[arg(value_name = "NODE_ID", required_unless_present = "all")]
    node_ids: Vec<String>,
}

fn read_run_add_toml(path: &PathBuf) -> BinResult<JsonValue> {
    let raw = fs::read_to_string(path)?;
    let toml_value: toml::Value = toml::from_str(&raw)?;
    let json_value = serde_json::to_value(toml_value)?;
    if !json_value.is_object() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "run-add TOML must be a table",
        )
        .into());
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

fn read_merged_run_add_toml(path: &PathBuf) -> BinResult<JsonValue> {
    let default_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_RUN_CONFIG_PATH);
    let mut merged = read_run_add_toml(&default_path)?;
    let overlay = read_run_add_toml(path)?;
    merge_json(&mut merged, overlay);
    Ok(merged)
}

fn parse_run_add_payload(raw: JsonValue) -> BinResult<(String, JsonValue, PointSpec)> {
    let mut root = raw.as_object().cloned().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "run-add payload must be a table",
        )
    })?;

    let name = root
        .remove("name")
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing run name (`name`) in run-add payload",
            )
        })
        .and_then(|value| {
            value
                .as_str()
                .map(|value| value.trim().to_string())
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid run name (`name`): expected non-empty string",
                    )
                })
        })?;
    if name.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid run name (`name`): expected non-empty string",
        )
        .into());
    }

    let point_spec_value = root.remove("point_spec").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing [point_spec] section in run-add payload",
        )
    })?;
    let point_spec: PointSpec = serde_json::from_value(point_spec_value).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid point_spec: {err}"),
        )
    })?;

    Ok((name, JsonValue::Object(root), point_spec))
}

fn print_assignment(assignment: &DesiredAssignment) {
    println!(
        "node={} role={} run_id={}",
        assignment.node_id, assignment.role, assignment.run_id
    );
}

#[tokio::main]
async fn main() -> BinResult {
    let cli = Cli::parse();

    let store = init_pg_store(10).await?;

    match cli.command {
        Command::Assign {
            node_id,
            role,
            run_id,
        } => {
            store
                .upsert_desired_assignment(&node_id, role.into(), run_id)
                .await?;
            println!(
                "assigned node={} role={} run_id={}",
                node_id,
                WorkerRole::from(role),
                run_id
            );
        }
        Command::Unassign { node_id, role } => {
            store
                .clear_desired_assignment(&node_id, WorkerRole::from(role))
                .await?;
            println!(
                "unassigned node={} role={}",
                node_id,
                WorkerRole::from(role)
            );
        }
        Command::ListAssignments { node_id } => {
            let assignments = store.list_desired_assignments(node_id.as_deref()).await?;
            if assignments.is_empty() {
                println!("no desired assignments");
            } else {
                for assignment in &assignments {
                    print_assignment(assignment);
                }
            }
        }
        Command::RunAdd {
            status,
            integration_params_file,
        } => {
            let (name, integration_params, point_spec) =
                parse_run_add_payload(read_merged_run_add_toml(&integration_params_file)?)?;
            let run_status: RunStatus = status.into();
            let run_id = store
                .create_run(run_status, &name, &integration_params, &point_spec)
                .await?;
            println!(
                "created run_id={} name={} status={}",
                run_id,
                name,
                run_status.as_str()
            );
        }
        Command::RunStart(selection) => {
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
        Command::RunPause(selection) => {
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
        Command::RunStop(selection) => {
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
        Command::RunRemove(selection) => {
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
        Command::NodeStop(selection) => {
            if selection.all {
                let rows = store.request_all_nodes_shutdown().await?;
                println!("requested shutdown for all nodes: rows_updated={rows}");
            } else {
                for node_id in selection.node_ids {
                    let rows = store.request_node_shutdown(&node_id).await?;
                    println!("requested shutdown for node={node_id}: rows_updated={rows}");
                }
            }
        }
    }

    Ok(())
}
