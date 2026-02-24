use clap::Parser;
use gammaboard::runners::{NodeRunner, NodeRunnerConfig};
use gammaboard::telemetry::init_tracing;
use gammaboard::{BinResult, init_pg_store};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "run_node")]
#[command(
    about = "Node-local reconciliation runner (role selected from DB desired assignment)",
    long_about = None
)]
struct Cli {
    #[arg(long)]
    node_id: String,
    #[arg(long, default_value_t = 1000)]
    poll_ms: u64,
}

#[tokio::main]
async fn main() -> BinResult {
    let cli = Cli::parse();

    let store = init_pg_store(10).await?;
    init_tracing(Some(store.pool().clone()))?;
    let node_runner = NodeRunner::new(
        store,
        cli.node_id,
        NodeRunnerConfig {
            poll_interval: Duration::from_millis(cli.poll_ms),
        },
    );
    node_runner.run().await?;

    Ok(())
}
