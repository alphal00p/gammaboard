use clap::Parser;
use gammaboard::{NodeWorkerConfig, PgStore, get_pg_pool, run_node_worker};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(name = "worker")]
#[command(about = "Node-local role reconciliation worker", long_about = None)]
struct Cli {
    #[arg(short = 't', long = "test")]
    test: bool,
    #[arg(long)]
    node_id: String,
    #[arg(long, default_value_t = 1000)]
    poll_ms: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if !cli.test {
        todo!("non-test engines are not wired yet; use --test / -t for now");
    }

    let pool = get_pg_pool(10).await?;
    let store = PgStore::new(pool);
    run_node_worker(
        store,
        cli.node_id,
        NodeWorkerConfig {
            poll_interval: Duration::from_millis(cli.poll_ms),
        },
    )
    .await?;

    Ok(())
}
