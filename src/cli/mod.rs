pub mod auth;
pub mod auto_assign;
pub mod completion;
pub mod db;
pub mod deploy;
pub mod node;
pub mod run;
pub mod server;
pub mod shared;

use anyhow::Result;
use auth::{AuthArgs, run_auth_hash_command};
use auto_assign::{AutoAssignArgs, run_auto_assign_command};
use clap::{ArgAction, Parser, Subcommand};
use completion::{CompletionArgs, run_completion};
use db::{DbArgs, run_db_command};
use deploy::{DeployArgs, run_deploy_command};
use gammaboard::config::{DEFAULT_RUNTIME_CONFIG_PATH, RuntimeConfig};
use node::{NodeArgs, run_node_commands};
use run::{RunArgs, run_run_commands};
use server::{ServerArgs, run_server};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gammaboard")]
#[command(about = "Gammaboard operations CLI", long_about = None)]
pub struct Cli {
    #[arg(long = "runtime-config", global = true, default_value = DEFAULT_RUNTIME_CONFIG_PATH, value_name = "PATH")]
    runtime_config: PathBuf,
    #[arg(short = 'q', long, global = true, action = ArgAction::SetTrue)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Assign free nodes to a run automatically
    AutoAssign(AutoAssignArgs),
    /// Run lifecycle commands
    Run(RunArgs),
    /// Node assignment and node lifecycle commands
    Node(NodeArgs),
    /// API server
    Server(ServerArgs),
    /// Auth helpers
    Auth(AuthArgs),
    /// Generate shell completion scripts
    Completion(CompletionArgs),
    /// Local PostgreSQL lifecycle helpers
    Db(DbArgs),
    /// Detached local deploy stack management
    Deploy(DeployArgs),
}

pub async fn dispatch(cli: Cli) -> Result<()> {
    let quiet = cli.quiet;
    let runtime_config_path = cli.runtime_config.clone();
    let config = RuntimeConfig::load(&runtime_config_path)?;
    match cli.command {
        Command::AutoAssign(args) => run_auto_assign_command(args, &config, quiet).await,
        Command::Run(args) => run_run_commands(args.command, &config, quiet).await,
        Command::Node(args) => {
            run_node_commands(args.command, &config, runtime_config_path.as_path(), quiet).await
        }
        Command::Server(args) => {
            run_server(args, &config, runtime_config_path.as_path(), quiet).await
        }
        Command::Auth(args) => run_auth_hash_command(args),
        Command::Completion(args) => run_completion(args),
        Command::Db(args) => run_db_command(args, &config),
        Command::Deploy(args) => run_deploy_command(args, &config, runtime_config_path.as_path()),
    }
}
