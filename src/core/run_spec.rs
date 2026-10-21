use crate::evaluation::{
    GammaLoopParams, ProcessEvaluatorParams, SymbolicaParams, UnitEvaluatorParams,
};
use crate::utils::domain::Domain;

use crate::core::tasks::DiscreteProjectionConfig;
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::sampling::HavanaInferenceSamplerParams;
use crate::sampling::{
    HavanaSamplerParams, NaiveMonteCarloSamplerParams, RasterLineSamplerParams,
    RasterPlaneSamplerParams, SphericalBatchTransformParams, UnitBallBatchTransformParams,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

pub type CapabilityRequirements = BTreeMap<String, u64>;

/// Canonical integration parameters payload stored on `runs.integration_params`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator: Option<EvaluatorConfig>,
    #[serde(default)]
    pub evaluator_requirements: CapabilityRequirements,
    #[serde(default)]
    pub sampler_requirements: CapabilityRequirements,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

/// Immutable run configuration loaded from storage before starting a run loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: i32,
    pub domain: Domain,
    pub integration_params: IntegrationParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum AccumulatorMomentConfig {
    #[default]
    MaxOrder2,
    MaxOrder4,
}

impl AccumulatorMomentConfig {
    pub fn max_order(self) -> u8 {
        self.into()
    }
}

impl From<AccumulatorMomentConfig> for u8 {
    fn from(value: AccumulatorMomentConfig) -> Self {
        match value {
            AccumulatorMomentConfig::MaxOrder2 => 2,
            AccumulatorMomentConfig::MaxOrder4 => 4,
        }
    }
}

impl TryFrom<u8> for AccumulatorMomentConfig {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            2 => Ok(Self::MaxOrder2),
            4 => Ok(Self::MaxOrder4),
            _ => Err(format!("accumulator moments must be 2 or 4, got {value}")),
        }
    }
}

/// Canonical accumulator model. Scalar input syntax is normalized to one component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccumulatorConfig {
    Empty,
    Vector {
        components: Vec<String>,
        training_projection: TrainingProjection,
        discrete_projections: Option<DiscreteProjectionConfig>,
        moments: AccumulatorMomentConfig,
    },
    Gammaloop,
    FullVector {
        components: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TrainingProjection {
    Component { name: String },
    Norm,
}

impl TrainingProjection {
    pub fn component(name: impl Into<String>) -> Self {
        Self::Component { name: name.into() }
    }
}

// Bincode needs externally tagged enums; unlike a kind plus optional fields,
// these representations cannot omit the data required by their variant.
#[derive(Serialize, Deserialize)]
enum BinaryProjection {
    Component(String),
    Norm,
}
#[derive(Serialize, Deserialize)]
enum BinaryAccumulatorConfig {
    Empty,
    Vector {
        components: Vec<String>,
        training_projection: BinaryProjection,
        discrete_projections: Option<DiscreteProjectionConfig>,
        moments: AccumulatorMomentConfig,
    },
    Gammaloop,
    FullVector {
        components: Vec<String>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AccumulatorInput {
    Empty,
    Scalar {
        discrete_projections: Option<DiscreteProjectionConfig>,
        #[serde(default)]
        moments: AccumulatorMomentConfig,
    },
    Vector {
        #[serde(default = "scalar_components")]
        components: Vec<String>,
        training_projection: Option<TrainingProjection>,
        #[serde(skip_serializing_if = "Option::is_none")]
        discrete_projections: Option<DiscreteProjectionConfig>,
        #[serde(default)]
        moments: AccumulatorMomentConfig,
    },
    Gammaloop,
    FullVector {
        #[serde(default = "scalar_components")]
        components: Vec<String>,
    },
}

fn scalar_components() -> Vec<String> {
    vec!["value".to_string()]
}

impl AccumulatorConfig {
    pub fn scalar() -> Self {
        Self::vector(scalar_components(), TrainingProjection::component("value"))
    }
    pub fn vector(components: Vec<String>, training_projection: TrainingProjection) -> Self {
        Self::Vector {
            components,
            training_projection,
            discrete_projections: None,
            moments: Default::default(),
        }
    }
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Vector { .. } => "vector",
            Self::Gammaloop => "gammaloop",
            Self::FullVector { .. } => "full_vector",
        }
    }
    pub fn components(&self) -> Option<&[String]> {
        match self {
            Self::Vector { components, .. } | Self::FullVector { components } => Some(components),
            _ => None,
        }
    }
    pub fn is_scalar(&self) -> bool {
        self.components().is_some_and(|c| c == ["value"])
    }
    pub fn discrete_projections(&self) -> Option<&DiscreteProjectionConfig> {
        match self {
            Self::Vector {
                discrete_projections,
                ..
            } => discrete_projections.as_ref(),
            _ => None,
        }
    }
    pub fn moments(&self) -> AccumulatorMomentConfig {
        match self {
            Self::Vector { moments, .. } => *moments,
            _ => Default::default(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(components) = self.components() {
            if components.is_empty() {
                return Err("accumulator components must not be empty".into());
            }
            let mut seen = std::collections::BTreeSet::new();
            for name in components {
                if name.trim().is_empty() || name == "training_projection" || !seen.insert(name) {
                    return Err("accumulator component names must be non-empty, unique, and not 'training_projection'".into());
                }
            }
        }
        if let Self::Vector {
            components,
            training_projection,
            discrete_projections,
            ..
        } = self
        {
            if let TrainingProjection::Component { name } = training_projection
                && !components.contains(name)
            {
                return Err(format!(
                    "training projection references unknown component {name:?}"
                ));
            }
            if let Some(config) = discrete_projections {
                config.validate()?;
                for stream in &config.streams {
                    if stream != "training_projection" && !components.contains(stream) {
                        return Err(format!(
                            "discrete_projections.streams references unknown vector stream '{stream}'"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

impl Serialize for AccumulatorConfig {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            match self {
                Self::Empty => AccumulatorInput::Empty,
                Self::Gammaloop => AccumulatorInput::Gammaloop,
                Self::FullVector { components } => AccumulatorInput::FullVector {
                    components: components.clone(),
                },
                Self::Vector {
                    components,
                    training_projection,
                    discrete_projections,
                    moments,
                } => AccumulatorInput::Vector {
                    components: components.clone(),
                    training_projection: Some(training_projection.clone()),
                    discrete_projections: discrete_projections.clone(),
                    moments: *moments,
                },
            }
            .serialize(serializer)
        } else {
            match self {
                Self::Empty => BinaryAccumulatorConfig::Empty,
                Self::Gammaloop => BinaryAccumulatorConfig::Gammaloop,
                Self::FullVector { components } => BinaryAccumulatorConfig::FullVector {
                    components: components.clone(),
                },
                Self::Vector {
                    components,
                    training_projection,
                    discrete_projections,
                    moments,
                } => BinaryAccumulatorConfig::Vector {
                    components: components.clone(),
                    training_projection: match training_projection {
                        TrainingProjection::Component { name } => {
                            BinaryProjection::Component(name.clone())
                        }
                        TrainingProjection::Norm => BinaryProjection::Norm,
                    },
                    discrete_projections: discrete_projections.clone(),
                    moments: *moments,
                },
            }
            .serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for AccumulatorConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let config = if deserializer.is_human_readable() {
            #[derive(Deserialize)]
            #[serde(untagged)]
            enum Input {
                Named(String),
                Rich(AccumulatorInput),
            }
            let input = match Input::deserialize(deserializer)? {
                Input::Rich(input) => input,
                Input::Named(name) => serde_json::from_value(serde_json::json!({"kind": name}))
                    .map_err(serde::de::Error::custom)?,
            };
            match input {
                AccumulatorInput::Empty => Self::Empty,
                AccumulatorInput::Gammaloop => Self::Gammaloop,
                AccumulatorInput::FullVector { components } => Self::FullVector { components },
                AccumulatorInput::Scalar {
                    discrete_projections,
                    moments,
                } => Self::Vector {
                    components: scalar_components(),
                    training_projection: TrainingProjection::component("value"),
                    discrete_projections,
                    moments,
                },
                AccumulatorInput::Vector {
                    components,
                    training_projection,
                    discrete_projections,
                    moments,
                } => {
                    let training_projection = match training_projection {
                        Some(projection) => projection,
                        None => TrainingProjection::component(
                            components
                                .first()
                                .ok_or_else(|| {
                                    serde::de::Error::custom(
                                        "accumulator components must not be empty",
                                    )
                                })?
                                .clone(),
                        ),
                    };
                    Self::Vector {
                        components,
                        training_projection,
                        discrete_projections,
                        moments,
                    }
                }
            }
        } else {
            match BinaryAccumulatorConfig::deserialize(deserializer)? {
                BinaryAccumulatorConfig::Empty => Self::Empty,
                BinaryAccumulatorConfig::Gammaloop => Self::Gammaloop,
                BinaryAccumulatorConfig::FullVector { components } => {
                    Self::FullVector { components }
                }
                BinaryAccumulatorConfig::Vector {
                    components,
                    training_projection,
                    discrete_projections,
                    moments,
                } => Self::Vector {
                    components,
                    training_projection: match training_projection {
                        BinaryProjection::Component(name) => TrainingProjection::component(name),
                        BinaryProjection::Norm => TrainingProjection::Norm,
                    },
                    discrete_projections,
                    moments,
                },
            }
        };
        config.validate().map_err(serde::de::Error::custom)?;
        Ok(config)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvaluatorConfig {
    Gammaloop {
        #[serde(flatten)]
        params: GammaLoopParams,
    },
    Unit {
        #[serde(flatten)]
        params: UnitEvaluatorParams,
    },
    Symbolica {
        #[serde(flatten)]
        params: SymbolicaParams,
    },
    ProcessEvaluator {
        #[serde(flatten)]
        params: ProcessEvaluatorParams,
    },
}

/// Complete sampling pipeline. Algorithm state belongs to the sampler; the
/// runtime owns materialization and coordinate transforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingConfig {
    #[serde(flatten)]
    pub sampler: SamplerConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materializer: Option<MaterializerConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transforms: Option<Vec<BatchTransformConfig>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SamplerConfig {
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
    PdfAdaptationRasterPlane {
        #[serde(flatten)]
        params: RasterPlaneSamplerParams,
    },
    PdfAdaptationRasterLine {
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
    ProcessSampler {
        #[serde(flatten)]
        params: crate::ProcessConfig,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaterializerConfig {
    ProcessMaterializer {
        #[serde(flatten)]
        params: crate::ProcessConfig,
    },
}

#[cfg(test)]
mod tests {
    use super::{AccumulatorConfig, AccumulatorMomentConfig};

    #[test]
    fn scalar_syntax_is_canonical_and_binary_config_is_validated() {
        for value in [
            serde_json::json!("scalar"),
            serde_json::json!({"kind":"scalar"}),
            serde_json::json!({"kind":"vector", "components":["value"]}),
        ] {
            let config: AccumulatorConfig = serde_json::from_value(value).unwrap();
            assert_eq!(config, AccumulatorConfig::scalar());
            let bytes =
                bincode::serde::encode_to_vec(&config, bincode::config::standard()).unwrap();
            let (restored, _): (AccumulatorConfig, usize) =
                bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
            assert_eq!(restored, config);
        }
        for components in [
            vec![],
            vec!["value".into(), "value".into()],
            vec!["other".into()],
        ] {
            let invalid = super::BinaryAccumulatorConfig::Vector {
                components,
                training_projection: super::BinaryProjection::Component("value".into()),
                discrete_projections: None,
                moments: AccumulatorMomentConfig::default(),
            };
            let bytes =
                bincode::serde::encode_to_vec(invalid, bincode::config::standard()).unwrap();
            assert!(
                bincode::serde::decode_from_slice::<AccumulatorConfig, _>(
                    &bytes,
                    bincode::config::standard()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn moment_defaults_and_explicit_order_are_identical() {
        let default: AccumulatorConfig = toml::from_str("kind = 'scalar'").unwrap();
        let explicit: AccumulatorConfig = toml::from_str("kind = 'scalar'\nmoments = 2").unwrap();
        assert_eq!(default, explicit);
        assert_eq!(
            serde_json::to_value(default).unwrap(),
            serde_json::to_value(explicit).unwrap()
        );
        for order in [2_u8, 4] {
            let config = AccumulatorMomentConfig::try_from(order).unwrap();
            assert_eq!(
                serde_json::to_value(config).unwrap(),
                serde_json::json!(order)
            );
            let bytes = bincode::serde::encode_to_vec(config, bincode::config::standard()).unwrap();
            let (restored, _): (AccumulatorMomentConfig, usize) =
                bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
            assert_eq!(restored, config);
        }
        for invalid in ["0", "3", "'default'", "{ max_order = 2 }"] {
            assert!(
                toml::from_str::<AccumulatorConfig>(&format!(
                    "kind = 'scalar'\nmoments = {invalid}"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn accumulator_config_parses_discrete_projections() {
        let config: AccumulatorConfig = toml::from_str(
            r#"
kind = "scalar"

[discrete_projections]
normalization = "conditional_mean"

[[discrete_projections.items]]
name = "channel_for_spin_0"
dims = [1]
fixed_dims = { "0" = 0 }
"#,
        )
        .expect("accumulator config");

        let AccumulatorConfig::Vector {
            discrete_projections: Some(projections),
            ..
        } = config
        else {
            panic!("expected scalar projection config");
        };
        assert_eq!(projections.items[0].fixed_dims.get("0"), Some(&0));
        assert_eq!(
            projections.normalization,
            crate::core::DiscreteProjectionNormalization::ConditionalMean
        );
    }

    #[test]
    fn accumulator_config_rejects_legacy_discrete_projection_bin_limit() {
        toml::from_str::<AccumulatorConfig>(
            r#"
kind = "scalar"

[discrete_projections]
max_total_bins = 16

[[discrete_projections.items]]
name = "spin"
dims = [0]
"#,
        )
        .expect_err("legacy max_total_bins should be rejected");
    }

    #[test]
    fn accumulator_config_parses_moment_config() {
        let config: AccumulatorConfig = toml::from_str(
            r#"
kind = "scalar"
moments = 4
"#,
        )
        .expect("accumulator config");

        assert_eq!(config.moments(), AccumulatorMomentConfig::MaxOrder4);
    }

    #[test]
    fn vector_accumulator_config_parses_moment_config() {
        let config: AccumulatorConfig = toml::from_str(
            r#"
kind = "vector"
components = ["real", "imag"]
training_projection = { kind = "component", name = "real" }
moments = 4
"#,
        )
        .expect("vector accumulator config");

        assert_eq!(config.moments(), AccumulatorMomentConfig::MaxOrder4);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BatchTransformConfig {
    UnitBall {
        #[serde(flatten)]
        params: UnitBallBatchTransformParams,
    },
    Spherical {
        #[serde(flatten)]
        params: SphericalBatchTransformParams,
    },
    ProcessBatchTransform {
        #[serde(flatten)]
        params: crate::ProcessConfig,
    },
}

impl<'de> Deserialize<'de> for EvaluatorConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Decode the tagged variant directly, so its parameter struct can reject
        // unknown fields without nested serde flatten losing field ownership.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Input {
            Gammaloop(GammaLoopParams),
            Unit(UnitEvaluatorParams),
            Symbolica(SymbolicaParams),
            ProcessEvaluator(ProcessEvaluatorParams),
        }
        Ok(match Input::deserialize(deserializer)? {
            Input::Gammaloop(params) => Self::Gammaloop { params },
            Input::Unit(params) => Self::Unit { params },
            Input::Symbolica(params) => Self::Symbolica { params },
            Input::ProcessEvaluator(params) => Self::ProcessEvaluator { params },
        })
    }
}

impl<'de> Deserialize<'de> for SamplerConfig {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Decode the tagged variant directly, so its parameter struct can reject
        // unknown fields without nested serde flatten losing field ownership.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Input {
            NaiveMonteCarlo(NaiveMonteCarloSamplerParams),
            RasterPlane(RasterPlaneSamplerParams),
            RasterLine(RasterLineSamplerParams),
            PdfAdaptationRasterPlane(RasterPlaneSamplerParams),
            PdfAdaptationRasterLine(RasterLineSamplerParams),
            HavanaTraining(HavanaSamplerParams),
            HavanaInference(HavanaInferenceSamplerParams),
            ProcessSampler(crate::ProcessConfig),
        }
        Ok(match Input::deserialize(deserializer)? {
            Input::NaiveMonteCarlo(params) => Self::NaiveMonteCarlo { params },
            Input::RasterPlane(params) => Self::RasterPlane { params },
            Input::RasterLine(params) => Self::RasterLine { params },
            Input::PdfAdaptationRasterPlane(params) => Self::PdfAdaptationRasterPlane { params },
            Input::PdfAdaptationRasterLine(params) => Self::PdfAdaptationRasterLine { params },
            Input::HavanaTraining(params) => Self::HavanaTraining { params },
            Input::HavanaInference(params) => Self::HavanaInference { params },
            Input::ProcessSampler(params) => Self::ProcessSampler { params },
        })
    }
}
