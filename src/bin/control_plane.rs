use clap::{Parser, Subcommand, ValueEnum};
use gammaboard::batch::PointSpec;
use gammaboard::core::{ControlPlaneStore, DesiredAssignment, RunStatus, WorkerRole};
use gammaboard::{BinResult, init_pg_store};
use serde_json::{Value as JsonValue, json};
use std::{fs, path::PathBuf};

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
        #[arg(long)]
        node_id: String,
        #[arg(long)]
        role: RoleArg,
        #[arg(long)]
        run_id: i32,
    },
    Unassign {
        #[arg(long)]
        node_id: String,
        #[arg(long)]
        role: RoleArg,
    },
    ListAssignments {
        #[arg(long)]
        node_id: Option<String>,
    },
    RunAdd {
        #[arg(long, value_enum, default_value_t = RunStatusArg::Pending)]
        status: RunStatusArg,
        #[arg(long)]
        integration_params_file: Option<PathBuf>,
    },
    RunStart {
        #[arg(long)]
        run_id: i32,
    },
    RunStop {
        #[arg(long)]
        run_id: i32,
        #[arg(long, value_enum, default_value_t = RunStatusArg::Paused)]
        status: RunStatusArg,
    },
    RunRemove {
        #[arg(long)]
        run_id: i32,
    },
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
            let (name, integration_params, point_spec) = match integration_params_file {
                Some(path) => parse_run_add_payload(read_run_add_toml(&path)?)?,
                None => parse_run_add_payload(json!({
                    "name": "live-test",
                    "point_spec": {
                        "continuous_dims": 1,
                        "discrete_dims": 0
                    },
                    "evaluator_implementation": "test_only_sin",
                    "evaluator_params": {},
                    "sampler_aggregator_implementation": "test_only_training",
                    "sampler_aggregator_params": {
                        "continuous_dims": 1,
                        "discrete_dims": 0,
                        "training_target_samples": 0,
                        "training_delay_per_sample_ms": 0
                    },
                    "observable_implementation": "scalar",
                    "observable_params": {},
                    "evaluator_runner_params": {
                        "min_loop_time_ms": 200,
                        "performance_snapshot_interval_ms": 5000
                    },
                    "sampler_aggregator_runner_params": {
                        "interval_ms": 500,
                        "lease_ttl_ms": 5000,
                        "nr_samples": 64,
                        "performance_snapshot_interval_ms": 5000,
                        "max_batches_per_tick": 1,
                        "max_pending_batches": 128,
                        "completed_batch_fetch_limit": 512
                    },
                }))?,
            };
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
        Command::RunStart { run_id } => {
            store.set_run_status(run_id, RunStatus::Running).await?;
            println!("run {} set to running", run_id);
        }
        Command::RunStop { run_id, status } => {
            let run_status: RunStatus = status.into();
            store.set_run_status(run_id, run_status).await?;
            println!("run {} set to {}", run_id, run_status.as_str());
        }
        Command::RunRemove { run_id } => {
            store.remove_run(run_id).await?;
            println!("removed run {}", run_id);
        }
    }

    Ok(())
}
