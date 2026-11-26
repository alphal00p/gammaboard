//! Offline database commands share the client grammar and require an instance file.
use clap::Args;
pub use gammaboard_client::cli::LocalDb as DbCommand;
#[derive(Debug, Args)]
pub struct DbArgs {
    #[command(subcommand)]
    pub command: DbCommand,
}
