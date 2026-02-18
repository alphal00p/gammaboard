use clap::{Parser, Subcommand, ValueEnum};
use gammaboard::{WorkerRole, ControlPlaneStore, DesiredAssignment, PgStore, get_pg_pool};
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
        #[arg(long, default_value = "pending")]
        status: String,
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
        #[arg(long, default_value = "paused")]
        status: String,
    },
    RunRemove {
        #[arg(long)]
        run_id: i32,
    },
}

fn read_integration_params_toml(path: &PathBuf) -> Result<JsonValue, Box<dyn std::error::Error>> {
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
    let role = match assignment.role {
        WorkerRole::Evaluator => "evaluator",
        WorkerRole::SamplerAggregator => "sampler_aggregator",
    };
    println!(
        "node={} role={} run_id={}",
        assignment.node_id, role, assignment.run_id
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let pool = get_pg_pool(10).await?;
    let store = PgStore::new(pool);

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
                "assigned node={} role={:?} run_id={}",
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
                "unassigned node={} role={:?}",
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
                None => json!({}),
            };
            let run_id = store.create_run(&status, &integration_params).await?;
            println!("created run_id={} status={}", run_id, status);
        }
        Command::RunStart { run_id } => {
            store.set_run_status(run_id, "running").await?;
            println!("run {} set to running", run_id);
        }
        Command::RunStop { run_id, status } => {
            store.set_run_status(run_id, &status).await?;
            println!("run {} set to {}", run_id, status);
        }
        Command::RunRemove { run_id } => {
            store.remove_run(run_id).await?;
            println!("removed run {}", run_id);
        }
    }

    Ok(())
}
