use super::shared::print_json;
use anyhow::Result;
use clap::{Args, Subcommand};
use gammaboard::{benchmark, config::RuntimeConfig};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct BenchmarkArgs {
    #[command(subcommand)]
    pub command: BenchmarkCommand,
}
#[derive(Debug, Subcommand)]
pub enum BenchmarkCommand {
    /// Database path capacity (normally invoked by python -m benchmarks)
    Io { config: PathBuf },
    /// External process adapter overhead, without PostgreSQL
    Protocol { config: PathBuf },
}
pub async fn run(args: BenchmarkArgs, runtime: &RuntimeConfig) -> Result<()> {
    let result = match args.command {
        BenchmarkCommand::Io { config } => {
            benchmark::io::measure(
                &runtime.database.url,
                serde_json::from_slice(&std::fs::read(config)?)?,
            )
            .await?
        }
        BenchmarkCommand::Protocol { config } => {
            benchmark::protocol::measure(serde_json::from_slice(&std::fs::read(config)?)?)?
        }
    };
    print_json(&result);
    Ok(())
}
