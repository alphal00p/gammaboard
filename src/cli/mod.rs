pub mod auth;
pub mod auto_assign;
pub mod completion;
pub mod db;
pub mod node;
pub mod run;
pub mod run_node;
pub mod server;
pub mod shared;

use anyhow::Result;
use auth::{AuthArgs, run_auth_hash_command};
use auto_assign::{AutoAssignArgs, run_auto_assign_command};
use clap::{ArgAction, Parser, Subcommand};
use completion::{CompletionArgs, run_completion};
use db::{DbArgs, run_db_command};
use gammaboard::config::{CliConfig, DEFAULT_CLI_CONFIG_PATH};
use node::{NodeArgs, run_node_commands};
use run::{RunArgs, run_run_commands};
use run_node::{RunNodeArgs, run_node};
use server::{ServerArgs, run_server};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gammaboard")]
#[command(about = "Gammaboard operations CLI", long_about = None)]
pub struct Cli {
    #[arg(long, global = true, default_value = DEFAULT_CLI_CONFIG_PATH, value_name = "PATH")]
    cli_config: PathBuf,
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
    /// Node-local role reconciler
    RunNode(RunNodeArgs),
    /// API server
    Server(ServerArgs),
    /// Auth helpers
    Auth(AuthArgs),
    /// Generate shell completion scripts
    Completion(CompletionArgs),
    /// Local PostgreSQL lifecycle helpers
    Db(DbArgs),
}

pub async fn dispatch(cli: Cli) -> Result<()> {
    let quiet = cli.quiet;
    let config = CliConfig::load(&cli.cli_config)?;
    match cli.command {
        Command::AutoAssign(args) => run_auto_assign_command(args, &config, quiet).await,
        Command::Run(args) => run_run_commands(args.command, &config, quiet).await,
        Command::Node(args) => run_node_commands(args.command, &config, quiet).await,
        Command::RunNode(args) => run_node(args, &config, quiet).await,
        Command::Server(args) => run_server(args, &config, quiet).await,
        Command::Auth(args) => run_auth_hash_command(args),
        Command::Completion(args) => run_completion(args),
        Command::Db(args) => run_db_command(args, &config),
    }
}
