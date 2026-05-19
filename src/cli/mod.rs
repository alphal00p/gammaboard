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
use gammaboard::config::DEFAULT_RUNTIME_CONFIG_PATH;
use gammaboard::runtime_context::{RuntimeContext, RuntimeOverrides};
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
    #[arg(long = "database-url", global = true, value_name = "URL")]
    database_url: Option<String>,
    #[arg(long = "resource-root", global = true, value_name = "PATH")]
    resource_roots: Vec<String>,
    #[arg(long = "port-offset", global = true, default_value_t = 0)]
    port_offset: u16,
    #[arg(long = "postgres-public", global = true, action = ArgAction::SetTrue)]
    postgres_public: bool,
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
    /// API-only backend server
    Server(ServerArgs),
    /// Auth helpers
    Auth(AuthArgs),
    /// Generate shell completion scripts
    Completion(CompletionArgs),
    /// Local PostgreSQL lifecycle helpers
    Db(DbArgs),
    /// Foreground dashboard deploy stack supervisor
    Deploy(DeployArgs),
}

pub async fn dispatch(cli: Cli) -> Result<()> {
    let quiet = cli.quiet;
    let runtime = RuntimeContext::load(
        &cli.runtime_config,
        RuntimeOverrides {
            database_url: cli.database_url,
            resource_roots: cli.resource_roots,
            port_offset: cli.port_offset,
            postgres_public: cli.postgres_public,
        },
    )?;
    let config = runtime.runtime_config();
    match cli.command {
        Command::AutoAssign(args) => run_auto_assign_command(args, config, quiet).await,
        Command::Run(args) => run_run_commands(args.command, config, quiet).await,
        Command::Node(args) => run_node_commands(args.command, &runtime, quiet).await,
        Command::Server(args) => run_server(args, &runtime, quiet).await,
        Command::Auth(args) => run_auth_hash_command(args),
        Command::Completion(args) => run_completion(args),
        Command::Db(args) => run_db_command(args, config),
        Command::Deploy(args) => run_deploy_command(args, &runtime).await,
    }
}
