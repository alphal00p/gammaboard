use crate::evaluation::PointSpec;
use crate::evaluation::{
    GammaLoopParams, SinEvaluatorParams, SincEvaluatorParams, SymbolicaParams, UnitEvaluatorParams,
};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::sampling::{
    HavanaInferenceParametrizationParams, HavanaInferenceSamplerParams, HavanaSamplerParams,
    IdentityParametrizationParams, NaiveMonteCarloSamplerParams, RasterLineSamplerParams,
    RasterPlaneSamplerParams, SphericalParametrizationParams, UnitBallParametrizationParams,
};
use serde::{Deserialize, Serialize};

/// Canonical integration parameters payload stored on `runs.integration_params`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationParams {
    pub evaluator: EvaluatorConfig,
    pub observable: ObservableConfig,
    pub sampler_aggregator: SamplerAggregatorConfig,
    pub parametrization: ParametrizationConfig,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

/// Immutable run configuration loaded from storage before starting a run loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: i32,
    pub point_spec: PointSpec,
    pub evaluator: EvaluatorConfig,
    pub observable: ObservableConfig,
    pub sampler_aggregator: SamplerAggregatorConfig,
    pub parametrization: ParametrizationConfig,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservableConfig {
    Scalar,
    Complex,
    FullScalar,
    FullComplex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluatorConfig {
    Gammaloop {
        #[serde(flatten)]
        params: GammaLoopParams,
    },
    SinEvaluator {
        #[serde(flatten)]
        params: SinEvaluatorParams,
    },
    SincEvaluator {
        #[serde(flatten)]
        params: SincEvaluatorParams,
    },
    Unit {
        #[serde(flatten)]
        params: UnitEvaluatorParams,
    },
    Symbolica {
        #[serde(flatten)]
        params: SymbolicaParams,
    },
}

impl EvaluatorConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SamplerAggregatorConfig {
    NaiveMonteCarlo {
        #[serde(flatten)]
        params: NaiveMonteCarloSamplerParams,
    },
    RasterPlane {
        #[serde(flatten)]
        params: RasterPlaneSamplerParams,
    },
    RasterLine {
        #[serde(flatten)]
        params: RasterLineSamplerParams,
    },
    HavanaTraining {
        #[serde(flatten)]
        params: HavanaSamplerParams,
    },
    HavanaInference {
        #[serde(flatten)]
        params: HavanaInferenceSamplerParams,
    },
}

impl SamplerAggregatorConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParametrizationConfig {
    Identity {
        #[serde(flatten)]
        params: IdentityParametrizationParams,
    },
    UnitBall {
        #[serde(flatten)]
        params: UnitBallParametrizationParams,
    },
    Spherical {
        #[serde(flatten)]
        params: SphericalParametrizationParams,
    },
    HavanaInference {
        #[serde(flatten)]
        params: HavanaInferenceParametrizationParams,
    },
}

impl ParametrizationConfig {}
