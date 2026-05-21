//! Gammaboard - Adaptive Numerical Integration System
//!
//! This library provides database abstractions for distributed adaptive
//! numerical integration using PostgreSQL as a work queue.

pub mod api;
pub mod config;
pub mod core;
pub mod evaluation;
pub mod preprocess;
mod process_runtime;
mod process_worker;
pub mod resources;
pub mod runners;
pub mod runtime_context;
pub mod sampling;
pub mod server;
pub mod stores;
pub mod tracing;
pub mod utils;

pub use core::{
    AccumulatorMetricName, AccumulatorMetricSelector, AccumulatorSourceSpec, ImageDisplayMode,
    LineDisplayMode, LineRasterGeometry, Linspace, MeasurementMetricSpec, MeasurementMode,
    MeasurementResult, MeasurementSpec, MeasurementStopCondition, PlaneRasterGeometry,
    PlotAccumulatorKind, RunTask, RunTaskInput, RunTaskSpec, RunTaskState, SampleErrorProjection,
    SampleStopCondition, SampleTaskConfig, SamplerAggregatorSourceSpec, SourceRefSpec,
    canonical_task_toml,
};
pub use core::{BatchRecord, BatchStatus};
pub use core::{BuildError, EngineError, EvalError, StoreError};
pub use evaluation::{
    Accumulator, AccumulatorMetricValue, AccumulatorState, Batch, BatchError, BatchResult,
    BatchTransform, EmptyAccumulatorState, EvalBatchOptions, Evaluator, FullAccumulatorProgress,
    FullVectorAccumulatorState, GammaLoopAccumulatorDigest, GammaLoopAccumulatorState,
    IngestScalar, IngestVector, Materializer, NamedScalarAccumulator, Point,
    ScalarAccumulatorState, SemanticAccumulatorKind, VectorAccumulatorState,
    extract_accumulator_metric, extract_accumulator_metric_with_runtime, ingest_scalar_values,
};
pub use sampling::{
    LatentBatch, LatentBatchPayload, LatentBatchSpec, PdfPoint, SamplePlan, SamplerAggregator,
    SamplerAggregatorSnapshot, StageHandoff,
};
pub use stores::PgStore;
pub use stores::{RunProgress, TaskOutputSnapshot, TaskStageSnapshot, WorkQueueStats};
pub use stores::{get_pg_pool, init_pg_store};
pub use utils::domain::{Domain, DomainBranch};
pub type BinResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
