use crate::evaluation::PointSpec;
use crate::evaluation::{
    GammaLoopParams, SinEvaluatorParams, SincEvaluatorParams, SymbolicaParams, UnitEvaluatorParams,
};
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::sampling::{
    HavanaInferenceMaterializerParams, HavanaInferenceSamplerParams, HavanaSamplerParams,
    IdentityMaterializerParams, NaiveMonteCarloSamplerParams, RasterLineSamplerParams,
    RasterPlaneSamplerParams, SphericalBatchTransformParams, UnitBallBatchTransformParams,
};
use serde::{Deserialize, Serialize};

/// Canonical integration parameters payload stored on `runs.integration_params`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationParams {
    pub evaluator: EvaluatorConfig,
    pub sampler_aggregator: SamplerAggregatorConfig,
    pub materializer: MaterializerConfig,
    pub batch_transforms: Vec<BatchTransformConfig>,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

/// Immutable run configuration loaded from storage before starting a run loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: i32,
    pub point_spec: PointSpec,
    pub evaluator: EvaluatorConfig,
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

impl ObservableConfig {
    pub const fn semantic_kind(&self) -> crate::evaluation::SemanticObservableKind {
        match self {
            Self::Scalar | Self::FullScalar => crate::evaluation::SemanticObservableKind::Scalar,
            Self::Complex | Self::FullComplex => crate::evaluation::SemanticObservableKind::Complex,
        }
    }
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
pub enum MaterializerConfig {
    Identity {
        #[serde(flatten)]
        params: IdentityMaterializerParams,
    },
    HavanaInference {
        #[serde(flatten)]
        params: HavanaInferenceMaterializerParams,
    },
}

impl MaterializerConfig {
    pub fn identity_default() -> Self {
        Self::Identity {
            params: IdentityMaterializerParams::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BatchTransformConfig {
    UnitBall {
        #[serde(flatten)]
        params: UnitBallBatchTransformParams,
    },
    Spherical {
        #[serde(flatten)]
        params: SphericalBatchTransformParams,
    },
}
