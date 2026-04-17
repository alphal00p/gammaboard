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

pub use core::{
    AccumulatorSourceSpec, ImageDisplayMode, LineDisplayMode, LineRasterGeometry, Linspace,
    PlaneRasterGeometry, PlotAccumulatorKind, RunTask, RunTaskInput, RunTaskSpec, RunTaskState,
    SampleTaskConfig, SamplerAggregatorSourceSpec, SourceRefSpec, canonical_task_toml,
};
pub use core::{BatchRecord, BatchStatus};
pub use core::{BuildError, EngineError, EvalError, StoreError};
pub use evaluation::{
    Accumulator, AccumulatorState, Batch, BatchError, BatchResult, BatchTransform,
    ComplexAccumulatorState, ComplexSampleEvaluator, ComplexValue, ComplexValueEvaluator,
    EmptyAccumulatorState, EvalBatchOptions, Evaluator, FullAccumulatorProgress,
    FullComplexAccumulatorState, FullScalarAccumulatorState, GammaLoopAccumulatorDigest,
    GammaLoopAccumulatorState, IngestComplex, IngestScalar, Materializer, Point,
    ScalarAccumulatorState, ScalarSampleEvaluator, ScalarValueEvaluator, SemanticAccumulatorKind,
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
