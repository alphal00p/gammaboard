use clap::Args;
use gammaboard::core::StoreError;
use gammaboard::runners::{NodeRunner, NodeRunnerConfig};
use gammaboard::telemetry::init_tracing;
use gammaboard::{BinResult, init_pg_store};
use std::time::Duration;

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
    init_tracing(Some(store.pool().clone()))?;
    let node_runner = NodeRunner::new(
        store,
        args.node_id,
        NodeRunnerConfig {
            poll_interval: Duration::from_millis(args.poll_ms),
        },
    );
    node_runner
        .run()
        .await
        .map_err(|err: StoreError| err.into())
}
