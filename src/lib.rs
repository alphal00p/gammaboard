//! Gammaboard - Adaptive Numerical Integration System
//!
//! This library provides database abstractions for distributed adaptive
//! numerical integration using PostgreSQL as a work queue.

pub mod aggregator;
pub mod batch;
pub mod models;
pub mod queries;

use dotenvy::dotenv;
use sqlx::{PgPool, Pool, Postgres, postgres::PgPoolOptions};
use std::env;

pub use batch::{Batch, BatchRecord, BatchResults, BatchStatus, WeightedPoint};
pub use models::{AggregatedResult, RunProgress, WorkQueueStats};
pub use queries::{
    claim_batch, get_aggregated_results, get_all_runs, get_latest_aggregated_result,
    get_run_progress, get_work_queue_stats, health_check, insert_batch, submit_batch_results,
};

/// Create a PostgreSQL connection pool
///
/// Loads DATABASE_URL from environment (via .env file) and creates
/// a connection pool with the specified maximum number of connections.
///
/// # Arguments
/// * `max_connections` - Maximum number of concurrent database connections
///
/// # Example
/// ```no_run
/// use gammaboard::get_pg_pool;
///
/// #[tokio::main]
/// async fn main() -> Result<(), sqlx::Error> {
///     let pool = get_pg_pool(10).await?;
///     Ok(())
/// }
/// ```
pub async fn get_pg_pool(max_connections: u32) -> Result<Pool<Postgres>, sqlx::Error> {
    dotenv().ok();
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(&db_url)
        .await
}

/// Type alias for database pool
pub type DbPool = PgPool;
