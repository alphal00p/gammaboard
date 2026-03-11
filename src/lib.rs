//! Gammaboard - Adaptive Numerical Integration System
//!
//! This library provides database abstractions for distributed adaptive
//! numerical integration using PostgreSQL as a work queue.

pub mod core;
pub mod engines;
pub mod preprocess;
pub mod runners;
pub mod server;
pub mod stores;
pub mod tracing;

pub use core::{Batch, BatchError, BatchRecord, BatchResult, BatchStatus, PointSpec};
pub use core::{RunStatus, StoreError};
pub use engines::{BuildError, EngineError, EvalError};
pub use stores::PgStore;
pub use stores::{AggregatedResult, RunProgress, WorkQueueStats};
pub use stores::{get_pg_pool, init_pg_store};
pub type BinResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
