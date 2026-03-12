pub mod completion;
pub mod node;
pub mod run;
pub mod run_node;
pub mod server;
pub mod shared;

use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand};
use completion::{CompletionArgs, run_completion};
use node::{NodeArgs, run_node_commands};
use run::{RunArgs, run_run_commands};
use run_node::{RunNodeArgs, run_node};
use server::{ServerArgs, run_server};

#[derive(Debug, Parser)]
#[command(name = "gammaboard")]
#[command(about = "Gammaboard operations CLI", long_about = None)]
pub struct Cli {
    #[arg(short = 'q', long, global = true, action = ArgAction::SetTrue)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run lifecycle commands
    Run(RunArgs),
    /// Node assignment and node lifecycle commands
    Node(NodeArgs),
    /// Node-local role reconciler
    RunNode(RunNodeArgs),
    /// API server
    Server(ServerArgs),
    /// Generate shell completion scripts
    Completion(CompletionArgs),
}

pub async fn dispatch(cli: Cli) -> Result<()> {
    let quiet = cli.quiet;
    match cli.command {
        Command::Run(args) => run_run_commands(args.command, quiet).await,
        Command::Node(args) => run_node_commands(args.command, quiet).await,
        Command::RunNode(args) => run_node(args, quiet).await,
        Command::Server(args) => run_server(args, quiet).await,
        Command::Completion(args) => run_completion(args),
    }
}
