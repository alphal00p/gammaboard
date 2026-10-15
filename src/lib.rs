//! GammaBoard - Adaptive Numerical Integration System
//!
//! This library provides database abstractions for distributed adaptive
//! numerical integration using PostgreSQL as a work queue.
//!
//! The crate root exports the core extension traits, their value types and
//! construction configs. Built-in parameters live in [`evaluation`] and
//! [`sampling`]; orchestration and persistence APIs live in [`api`] and [`core`].
//!
//! ```
//! use gammaboard::{Batch, Domain, Point};
//!
//! let batch = Batch::from_iter([Point::new(vec![0.5], vec![], 1.0)]);
//! Domain::continuous(1).validate_batch(&batch).unwrap();
//! assert_eq!(batch.len(), 1);
//! ```

pub mod api;
pub mod benchmark;
pub mod compatibility;
pub mod config;
pub mod core;
pub mod evaluation;
#[doc(hidden)]
pub mod local_db;
pub mod preprocess;
mod process_runtime;
mod process_worker;
pub mod provenance;
pub mod resources;
pub mod runners;
pub mod runtime_context;
pub mod sampling;
pub mod server;
mod service_error;
mod services;
pub mod stores;
mod symbolica_license;
pub mod tracing;
pub mod utils;

pub use process_runtime::ProcessConfig;

pub use core::BatchStatus;
pub use core::{
    AccumulatorConfig, AccumulatorMomentConfig, BatchTransformConfig, EvaluatorConfig,
    IntegrationParams, MaterializerConfig, RunSpec, SamplerConfig, SamplingConfig,
    TrainingProjection,
};
pub use core::{
    AccumulatorMetricName, AccumulatorMetricSelector, AccumulatorSourceSpec, ControllerChildOutput,
    ControllerChildState, ControllerTaskOutput, DerivedResultSnapshot, EgoboxInfillStrategy,
    EgoboxOptimizerParams, EgoboxQeiStrategy, HyperparameterTrialOutput,
    HyperparameterTuningCategoricalDomain, HyperparameterTuningFloatDomain,
    HyperparameterTuningIntegerDomain, HyperparameterTuningOptimizerSpec,
    HyperparameterTuningOutput, HyperparameterTuningParameterDomain, ImageDisplayMode,
    IntegrationCampaignChildOutput, IntegrationCampaignOutput, LineDisplayMode, LineRasterGeometry,
    Linspace, MeasurementMode, MeasurementResult, MeasurementSelection, MeasurementSpec,
    ParameterIntegerRangeSpec, ParameterLinspaceSpec, ParameterScanOutput,
    ParameterScanParameterSpec, ParameterScanPointOutput, ParameterValueSourceSpec,
    PlaneRasterGeometry, PlotAccumulatorKind, ResultSourceRef, RunTask, RunTaskInput, RunTaskSpec,
    RunTaskState, SampleStopCondition, SamplerSourceSpec, SourceRefSpec, TaskMeasurementOutput,
    TaskMeasurementSpec, canonical_task_toml,
};
pub use core::{EngineError, StoreError};
pub use evaluation::{
    Accumulator, AccumulatorState, Batch, BatchError, BatchResult, BatchTransform,
    DiagnosticSample, EmptyAccumulatorState, EvalBatchOptions, Evaluator, FullAccumulatorProgress,
    FullVectorAccumulatorState, GammaLoopAccumulatorDigest, GammaLoopAccumulatorState,
    IngestScalar, IngestVector, Materializer, NamedScalarAccumulator, Point, SampleEvaluation,
    ScalarAccumulatorState, VectorAccumulatorState, WeightFactor, extract_accumulator_metric,
    extract_accumulator_metric_with_runtime, ingest_scalar_values,
};
pub use sampling::{
    DiscreteSubspace, Generation, LatentBatch, LatentBatchPayload, PdfPoint, Sampler, SavedSampler,
    StageHandoff, StageHandoffOwned,
};
pub use stores::PgStore;
pub use stores::{
    RunLifecycleState, RunProgress, TaskOutputSnapshot, TaskStageSnapshot, WorkerStatus,
};
pub use stores::{get_pg_pool, init_pg_store};
pub use symbolica_license::activate_symbolica_oem_license;
pub use utils::domain::{Domain, DomainBranch};
pub type BinResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[cfg(test)]
pub(crate) mod test_support;
