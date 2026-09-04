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
use auth::{AuthArgs, run_auth_command};
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
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    pub fn json_output_requested(&self) -> bool {
        self.json
    }
}

pub fn format_error_json(code: &str, message: impl std::fmt::Display) -> String {
    serde_json::json!({
        "error": {
            "code": code,
            "message": message.to_string(),
        }
    })
    .to_string()
}

#[derive(Debug, Subcommand)]
enum Command {
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
    shared::set_json_output(cli.json);
    if cli.postgres_public && !cli.json {
        eprintln!(
            "WARNING: --postgres-public exposes PostgreSQL on 0.0.0.0/0 with trust authentication; any host that can reach the port can connect without a password"
        );
    }
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
        Command::Run(args) => run_run_commands(args.command, config, quiet).await,
        Command::Node(args) => run_node_commands(args.command, &runtime, quiet).await,
        Command::Server(args) => run_server(args, &runtime, quiet).await,
        Command::Auth(args) => run_auth_command(args),
        Command::Completion(args) => run_completion(args),
        Command::Db(args) => run_db_command(args, config),
        Command::Deploy(args) => run_deploy_command(args, &runtime).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, format_error_json};
    use clap::{CommandFactory, Parser};

    #[test]
    fn public_command_tree_uses_release_names() {
        let command = Cli::command();
        let run = command.find_subcommand("run").expect("run command");
        assert!(run.find_subcommand("create").is_some());
        assert!(run.find_subcommand("add").is_none());
        let task = run.find_subcommand("task").expect("run task command");
        assert!(task.find_subcommand("append").is_some());
        assert!(task.find_subcommand("add").is_none());

        let node = command.find_subcommand("node").expect("node command");
        assert!(node.find_subcommand("auto-assign").is_some());
        assert!(node.find_subcommand("start-local").is_some());
        assert!(node.find_subcommand("auto-run").is_none());
        assert!(command.find_subcommand("auto-assign").is_none());

        let auth = command.find_subcommand("auth").expect("auth command");
        assert!(auth.find_subcommand("hash-password").is_some());
        let deploy = command.find_subcommand("deploy").expect("deploy command");
        assert!(deploy.find_subcommand("run").is_none());
    }

    #[test]
    fn destructive_flags_are_limited_to_destructive_commands() {
        assert!(Cli::try_parse_from(["gammaboard", "run", "pause", "--dry-run", "demo"]).is_err());
        assert!(Cli::try_parse_from(["gammaboard", "run", "remove", "--dry-run", "demo"]).is_ok());
        assert!(
            Cli::try_parse_from(["gammaboard", "run", "task", "remove", "demo", "1", "--yes",])
                .is_ok()
        );
    }

    #[test]
    fn json_errors_have_a_stable_shape() {
        let payload: serde_json::Value =
            serde_json::from_str(&format_error_json("command_failed", "missing run"))
                .expect("error JSON");
        assert_eq!(payload["error"]["code"], "command_failed");
        assert_eq!(payload["error"]["message"], "missing run");
    }
}
