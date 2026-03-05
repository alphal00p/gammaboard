use anyhow::{Context, Result};
use clap::Args;
use gammaboard::init_pg_store;
use gammaboard::runners::{NodeRunner, NodeRunnerConfig};
use std::time::Duration;
use tracing::Instrument;

use super::shared::init_cli_tracing;

#[derive(Debug, Args)]
pub struct RunNodeArgs {
    #[arg(long)]
    node_id: String,
    #[arg(long, default_value_t = 1000)]
    poll_ms: u64,
    #[arg(long, default_value_t = 10)]
    db_pool_size: u32,
}

pub async fn run_node(args: RunNodeArgs) -> Result<()> {
    let store = init_pg_store(args.db_pool_size)
        .await
        .context("failed to initialize postgres store")?;
    init_cli_tracing(&store)?;
    let node_id = args.node_id.clone();
    let node_runner = NodeRunner::new(
        store,
        node_id.clone(),
        NodeRunnerConfig {
            poll_interval: Duration::from_millis(args.poll_ms),
        },
    );
    let span = tracing::span!(
        tracing::Level::TRACE,
        "run_node",
        source = "worker",
        node_id = %node_id
    );
    node_runner.run().instrument(span).await?;
    Ok(())
}
