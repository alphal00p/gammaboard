//! Gammaboard - Adaptive Numerical Integration System
//!
//! This library provides database abstractions for distributed adaptive
//! numerical integration using PostgreSQL as a work queue.

pub mod batch;
pub mod contracts;
pub mod control_plane;
pub mod engines;
pub mod models;
pub mod runners;
pub mod stores;

use dotenvy::dotenv;
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use std::env;

pub use batch::{Batch, BatchRecord, BatchResults, BatchStatus, WeightedPoint};
pub use contracts::{
    AggregationStore, AssignmentLease, AssignmentLeaseStore, BatchClaim, BuildError,
    CompletedBatch, Worker, WorkerRegistryStore, WorkerRole, ControlPlaneStore,
    DesiredAssignment, EngineError, EngineState, EngineStateStore, EvalError, Evaluator,
    EvaluatorFactory, WorkerStatus, RunReadStore, RunSpec, RunSpecStore, SamplerAggregatorEngine,
    SamplerAggregatorFactory, StoreError, WorkQueueStore,
};
pub use control_plane::{NodeWorkerConfig, run_node_worker};
pub use models::{AggregatedResult, RunProgress, WorkQueueStats};
pub use runners::{RunnerConfig, RunnerError, RunnerTick, SamplerAggregatorRunner};
pub use runners::{WorkerRunner, WorkerRunnerConfig, WorkerRunnerError, WorkerTick};
pub use stores::PgStore;

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
