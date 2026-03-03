pub mod node;
pub mod run;
pub mod run_node;
pub mod server;
pub mod shared;

use clap::{Parser, Subcommand};
use gammaboard::BinResult;
use node::{NodeArgs, run_node_commands};
use run::{RunArgs, run_run_commands};
use run_node::{RunNodeArgs, run_node};
use server::{ServerArgs, run_server};

#[derive(Debug, Parser)]
#[command(name = "gammaboard")]
#[command(about = "Gammaboard operations CLI", long_about = None)]
pub struct Cli {
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
}

pub async fn dispatch(cli: Cli) -> BinResult {
    match cli.command {
        Command::Run(args) => run_run_commands(args.command).await,
        Command::Node(args) => run_node_commands(args.command).await,
        Command::RunNode(args) => run_node(args).await,
        Command::Server(args) => run_server(args).await,
    }
}
