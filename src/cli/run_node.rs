use clap::Args;
use gammaboard::core::StoreError;
use gammaboard::runners::{NodeRunner, NodeRunnerConfig};
use gammaboard::{BinResult, init_pg_store};
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

pub async fn run_node(args: RunNodeArgs) -> BinResult {
    let store = init_pg_store(args.db_pool_size).await?;
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
    node_runner
        .run()
        .instrument(span)
        .await
        .map_err(|err: StoreError| err.into())
}
