//! Gammaboard - Adaptive Numerical Integration System
//!
//! This library provides database abstractions for distributed adaptive
//! numerical integration using PostgreSQL as a work queue.

pub mod batch;
pub mod contracts;
pub mod engines;
pub mod models;
pub mod runners;
pub mod stores;

use dotenvy::dotenv;
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use std::env;

pub use batch::{Batch, BatchRecord, BatchResults, BatchStatus, WeightedPoint};
pub use contracts::errors::{BuildError, EngineError, EvalError, StoreError};
pub use models::{AggregatedResult, RunProgress, RunStatus, WorkQueueStats};
pub use stores::PgStore;
pub type BinResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

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

/// Initialize a Postgres-backed store with the given max pool size.
pub async fn init_pg_store(max_connections: u32) -> Result<PgStore, sqlx::Error> {
    let pool = get_pg_pool(max_connections).await?;
    Ok(PgStore::new(pool))
}
