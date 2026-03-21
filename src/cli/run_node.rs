use super::shared::with_cli_store;
use anyhow::Result;
use clap::Args;
use gammaboard::runners::{NodeRunner, NodeRunnerConfig};

#[derive(Debug, Args)]
pub struct RunNodeArgs {
    #[arg(long)]
    name: String,
    #[arg(long, default_value_t = 3)]
    max_start_failures: u32,
    #[arg(long, default_value_t = 10)]
    db_pool_size: u32,
}

pub async fn run_node(args: RunNodeArgs, quiet: bool) -> Result<()> {
    let node_name = args.name.clone();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "run-node",
        source = "worker",
        node_name = %node_name
    );
    with_cli_store(args.db_pool_size, quiet, span, |store| async move {
        let node_runner = NodeRunner::new(
            store,
            node_name,
            NodeRunnerConfig {
                max_consecutive_start_failures: args.max_start_failures,
                ..NodeRunnerConfig::default()
            },
        );
        node_runner.run().await?;
        Ok(())
    })
    .await
}
