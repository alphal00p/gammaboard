pub mod accumulator;
pub(crate) mod batch;
pub(crate) mod evaluator;
pub mod stratification;
pub(crate) mod traits;

pub use accumulator::{
    Accumulator, AccumulatorState, EmptyAccumulatorState, FullAccumulatorProgress,
    FullVectorAccumulatorState, GammaLoopAccumulatorDigest, GammaLoopAccumulatorState,
    GammaLoopDiagnostics, IngestScalar, IngestVector, NamedScalarAccumulator,
    ScalarAccumulatorState, VectorAccumulatorState, extract_accumulator_metric,
    extract_accumulator_metric_with_runtime, relative_error,
};
pub use batch::{
    Batch, BatchError, BatchResult, DiagnosticSample, Point, SampleEvaluation, WeightFactor,
};
pub use evaluator::gammaloop::TrainingProjection as ObservableTrainingProjection;
pub use evaluator::{
    GammaLoopParams, ProcessEvaluatorParams, ProcessOutput, SymbolicaParams, UnitEvaluatorParams,
};
mod reduction;
pub use reduction::SampleOutput;
pub use traits::{BatchTransform, EvalBatchOptions, Evaluator, EvaluatorBatch, Materializer};
