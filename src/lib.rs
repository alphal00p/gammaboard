//! Gammaboard - Adaptive Numerical Integration System
//!
//! This library provides database abstractions for distributed adaptive
//! numerical integration using PostgreSQL as a work queue.

pub mod api;
pub mod config;
pub mod core;
pub mod evaluation;
pub mod preprocess;
pub mod runners;
pub mod sampling;
pub mod server;
pub mod stores;
pub mod tracing;
pub mod utils;

pub use core::{BatchRecord, BatchStatus};
pub use core::{BuildError, EngineError, EvalError, StoreError};
pub use core::{
    ImageDisplayMode, IntoPreflightTask, LineDisplayMode, LineRasterGeometry, Linspace,
    MaterializerState, PlaneRasterGeometry, PlotObservableKind, RunTask, RunTaskInputSpec,
    RunTaskSpec, RunTaskState, StageSnapshotRef, resolve_initial_sampler_aggregator,
    resolve_task_queue,
};
pub use evaluation::{
    Batch, BatchError, BatchResult, BatchTransform, ComplexObservableState, ComplexSampleEvaluator,
    ComplexValue, ComplexValueEvaluator, EvalBatchOptions, Evaluator, FullComplexObservableState,
    FullObservableProgress, FullScalarObservableState, IngestComplex, IngestScalar, Materializer,
    Observable, ObservableState, PointSpec, ScalarObservableState, ScalarSampleEvaluator,
    ScalarValueEvaluator, SemanticObservableKind,
};
pub use sampling::{
    LatentBatch, LatentBatchPayload, LatentBatchSpec, SamplePlan, SamplerAggregator,
    SamplerAggregatorSnapshot, StageHandoff,
};
pub use stores::PgStore;
pub use stores::{RunProgress, TaskOutputSnapshot, TaskStageSnapshot, WorkQueueStats};
pub use stores::{get_pg_pool, init_pg_store};
pub type BinResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
