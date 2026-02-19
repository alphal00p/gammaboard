use clap::{Parser, Subcommand, ValueEnum};
use gammaboard::contracts::{ControlPlaneStore, DesiredAssignment, WorkerRole};
use gammaboard::models::RunStatus;
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

fn read_integration_params_toml(path: &PathBuf) -> BinResult<JsonValue> {
    let raw = fs::read_to_string(path)?;
    let toml_value: toml::Value = toml::from_str(&raw)?;
    let json_value = serde_json::to_value(toml_value)?;
    if !json_value.is_object() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "integration params TOML must be a table",
        )
        .into());
    }
    Ok(json_value)
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
            let integration_params = match integration_params_file {
                Some(path) => read_integration_params_toml(&path)?,
                None => json!({
                    "evaluator_implementation": "test_only_sin",
                    "evaluator_params": {},
                    "sampler_aggregator_implementation": "test_only_training",
                    "sampler_aggregator_params": {},
                    "observable_implementation": "test_only",
                    "observable_params": {},
                    "worker_runner_params": {
                        "min_loop_time_ms": 200
                    },
                    "sampler_aggregator_runner_params": {
                        "interval_ms": 500,
                        "lease_ttl_ms": 5000,
                        "max_batches_per_tick": 1,
                        "max_pending_batches": 128,
                        "completed_batch_fetch_limit": 512
                    },
                }),
            };
            let run_status: RunStatus = status.into();
            let run_id = store.create_run(run_status, &integration_params).await?;
            println!("created run_id={} status={}", run_id, run_status.as_str());
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
