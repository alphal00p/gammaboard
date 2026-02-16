//! Sampler-aggregator scheduler binary.
//!
//! This binary is intentionally thin: it only handles process lifecycle and
//! scheduling. All database operations and aggregation logic live in the library
//! (`gammaboard::aggregator` and `gammaboard::queries`).

use gammaboard::{aggregator, get_pg_pool};
use std::{env, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧮 Starting sampler-aggregator...");

    let pool = get_pg_pool(5).await?;
    println!("✅ Connected to database");

    let interval_ms = env::var("AGGREGATOR_INTERVAL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    let run_filter = env::var("RUN_ID").ok().and_then(|v| v.parse::<i32>().ok());

    loop {
        let run_ids = if let Some(id) = run_filter {
            vec![id]
        } else {
            aggregator::list_run_ids(&pool).await?
        };

        for run_id in run_ids {
            match aggregator::aggregate_run(&pool, run_id).await {
                Ok(Some(snapshot)) => {
                    println!(
                        "📈 Run {} aggregated: total_samples={}, total_batches={}, mean={:?}, error={:?}",
                        run_id,
                        snapshot.nr_samples,
                        snapshot.nr_batches,
                        snapshot.mean,
                        snapshot.error_estimate
                    );
                }
                Ok(None) => {
                    // No new completed batches since last snapshot.
                }
                Err(err) => {
                    eprintln!("❌ Aggregation error for run {}: {}", run_id, err);
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(interval_ms)).await;
    }
}
