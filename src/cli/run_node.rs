use super::shared::with_cli_store;
use anyhow::Result;
use clap::Args;
use gammaboard::runners::{NodeRunner, NodeRunnerConfig};
use std::time::Duration;

#[derive(Debug, Args)]
pub struct RunNodeArgs {
    #[arg(long)]
    node_id: String,
    #[arg(long, default_value_t = 1000)]
    poll_ms: u64,
    #[arg(long, default_value_t = 3)]
    max_start_failures: u32,
    #[arg(long, default_value_t = 10)]
    db_pool_size: u32,
}

pub async fn run_node(args: RunNodeArgs, quiet: bool) -> Result<()> {
    let node_id = args.node_id.clone();
    let span = tracing::span!(
        tracing::Level::TRACE,
        "run-node",
        source = "worker",
        node_id = %node_id
    );
    with_cli_store(args.db_pool_size, quiet, span, |store| async move {
        let node_runner = NodeRunner::new(
            store,
            node_id,
            NodeRunnerConfig {
                min_tick_time: Duration::from_millis(args.poll_ms),
                max_consecutive_start_failures: args.max_start_failures,
            },
        );
        node_runner.run().await?;
        Ok(())
    })
    .await
}
