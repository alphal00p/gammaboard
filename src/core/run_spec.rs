use crate::evaluation::{
    GammaLoopParams, ProcessEvaluatorParams, SymbolicaParams, UnitEvaluatorParams,
};
use crate::utils::domain::Domain;

use crate::core::tasks::DiscreteProjectionConfig;
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::sampling::HavanaInferenceSamplerParams;
use crate::sampling::{
    HavanaSamplerParams, NaiveMonteCarloSamplerParams, ProcessBatchTransformParams,
    ProcessMaterializerParams, ProcessSamplerParams, RasterLineSamplerParams,
    RasterPlaneSamplerParams, SphericalBatchTransformParams, UnitBallBatchTransformParams,
};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;

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
    pub evaluator: Option<EvaluatorConfig>,
    pub evaluator_requirements: CapabilityRequirements,
    pub sampler_requirements: CapabilityRequirements,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccumulatorMomentConfig {
    #[default]
    Default,
    MaxOrder2,
    MaxOrder4,
}

impl AccumulatorMomentConfig {
    pub fn max_order(self) -> u8 {
        match self {
            Self::Default | Self::MaxOrder2 => 2,
            Self::MaxOrder4 => 4,
        }
    }

    pub fn validate(self) -> Result<(), String> {
        match self {
            Self::Default | Self::MaxOrder2 | Self::MaxOrder4 => Ok(()),
        }
    }
}

impl Serialize for AccumulatorMomentConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if !serializer.is_human_readable() {
            let encoded = match self {
                Self::Default => 0,
                Self::MaxOrder2 => 2,
                Self::MaxOrder4 => 4,
            };
            return serializer.serialize_u8(encoded);
        }
        if *self == Self::Default {
            serializer.serialize_str("default")
        } else {
            #[derive(Serialize)]
            struct MomentTable {
                max_order: u8,
            }
            MomentTable {
                max_order: self.max_order(),
            }
            .serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for AccumulatorMomentConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if !deserializer.is_human_readable() {
            let max_order = u8::deserialize(deserializer)?;
            return match max_order {
                0 => Ok(Self::Default),
                2 => Ok(Self::MaxOrder2),
                4 => Ok(Self::MaxOrder4),
                other => Err(serde::de::Error::custom(format!(
                    "accumulator moment max_order must be 2 or 4, got {other}"
                ))),
            };
        }
        struct MomentVisitor;

        impl<'de> Visitor<'de> for MomentVisitor {
            type Value = AccumulatorMomentConfig;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(r#""default" or a table like { max_order = 4 }"#)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "default" => Ok(AccumulatorMomentConfig::Default),
                    other => Err(E::unknown_variant(other, &["default"])),
                }
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct MomentTable {
                    max_order: u8,
                }
                match MomentTable::deserialize(serde::de::value::MapAccessDeserializer::new(map))?
                    .max_order
                {
                    2 => Ok(AccumulatorMomentConfig::MaxOrder2),
                    4 => Ok(AccumulatorMomentConfig::MaxOrder4),
                    other => Err(serde::de::Error::custom(format!(
                        "accumulator moments.max_order must be 2 or 4, got {other}"
                    ))),
                }
            }
        }

        deserializer.deserialize_any(MomentVisitor)
    }
}

fn is_default_moment_config(value: &AccumulatorMomentConfig) -> bool {
    *value == AccumulatorMomentConfig::default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccumulatorConfig {
    Empty,
    Scalar {
        discrete_projections: Option<DiscreteProjectionConfig>,
        moments: AccumulatorMomentConfig,
    },
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
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrainingProjection {
    Component { name: String },
    Norm,
}

impl TrainingProjection {
    pub fn component(name: impl Into<String>) -> Self {
        Self::Component { name: name.into() }
    }
}

impl From<TrainingProjection> for BinaryTrainingProjection {
    fn from(value: TrainingProjection) -> Self {
        match value {
            TrainingProjection::Component { name } => Self {
                kind: BinaryTrainingProjectionKind::Component,
                name: Some(name),
            },
            TrainingProjection::Norm => Self {
                kind: BinaryTrainingProjectionKind::Norm,
                name: None,
            },
        }
    }
}

impl From<BinaryTrainingProjection> for TrainingProjection {
    fn from(value: BinaryTrainingProjection) -> Self {
        match value.kind {
            BinaryTrainingProjectionKind::Component => {
                TrainingProjection::component(value.name.unwrap_or_else(|| "value".to_string()))
            }
            BinaryTrainingProjectionKind::Norm => TrainingProjection::Norm,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AccumulatorConfigKind {
    Empty,
    Scalar,
    Vector,
    Gammaloop,
    FullVector,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BinaryAccumulatorConfig {
    kind: AccumulatorConfigKind,
    discrete_projections: Option<DiscreteProjectionConfig>,
    #[serde(default)]
    moments: AccumulatorMomentConfig,
    components: Option<Vec<String>>,
    training_projection: Option<BinaryTrainingProjection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BinaryTrainingProjectionKind {
    Component,
    Norm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BinaryTrainingProjection {
    kind: BinaryTrainingProjectionKind,
    name: Option<String>,
}

impl AccumulatorConfig {
    pub fn scalar() -> Self {
        Self::Scalar {
            discrete_projections: None,
            moments: AccumulatorMomentConfig::default(),
        }
    }

    pub fn vector(components: Vec<String>, training_projection: TrainingProjection) -> Self {
        Self::Vector {
            components,
            training_projection,
            discrete_projections: None,
            moments: AccumulatorMomentConfig::default(),
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Scalar { .. } => "scalar",
            Self::Vector { .. } => "vector",
            Self::Gammaloop => "gammaloop",
            Self::FullVector { .. } => "full_vector",
        }
    }

    fn kind(&self) -> AccumulatorConfigKind {
        match self {
            Self::Empty => AccumulatorConfigKind::Empty,
            Self::Scalar { .. } => AccumulatorConfigKind::Scalar,
            Self::Vector { .. } => AccumulatorConfigKind::Vector,
            Self::Gammaloop => AccumulatorConfigKind::Gammaloop,
            Self::FullVector { .. } => AccumulatorConfigKind::FullVector,
        }
    }

    fn to_binary(&self) -> BinaryAccumulatorConfig {
        BinaryAccumulatorConfig {
            kind: self.kind(),
            discrete_projections: self.discrete_projections().cloned(),
            moments: self.moments(),
            components: match self {
                Self::Vector { components, .. } | Self::FullVector { components } => {
                    Some(components.clone())
                }
                _ => None,
            },
            training_projection: match self {
                Self::Vector {
                    training_projection,
                    ..
                } => Some(BinaryTrainingProjection::from(training_projection.clone())),
                _ => None,
            },
        }
    }

    fn from_parts(
        kind: AccumulatorConfigKind,
        discrete_projections: Option<DiscreteProjectionConfig>,
        moments: AccumulatorMomentConfig,
        components: Option<Vec<String>>,
        training_projection: Option<TrainingProjection>,
    ) -> Self {
        match kind {
            AccumulatorConfigKind::Empty => Self::Empty,
            AccumulatorConfigKind::Scalar => Self::Scalar {
                discrete_projections,
                moments,
            },
            AccumulatorConfigKind::Vector => {
                let components = components.unwrap_or_else(|| vec!["value".to_string()]);
                let training_projection = training_projection
                    .unwrap_or_else(|| TrainingProjection::component(components[0].clone()));
                Self::Vector {
                    components,
                    training_projection,
                    discrete_projections,
                    moments,
                }
            }
            AccumulatorConfigKind::Gammaloop => Self::Gammaloop,
            AccumulatorConfigKind::FullVector => Self::FullVector {
                components: components.unwrap_or_else(|| vec!["value".to_string()]),
            },
        }
    }

    fn kind_accepts_discrete_projections(kind: AccumulatorConfigKind) -> bool {
        matches!(
            kind,
            AccumulatorConfigKind::Scalar | AccumulatorConfigKind::Vector
        )
    }

    pub fn discrete_projections(&self) -> Option<&DiscreteProjectionConfig> {
        match self {
            Self::Scalar {
                discrete_projections,
                ..
            }
            | Self::Vector {
                discrete_projections,
                ..
            } => discrete_projections.as_ref(),
            Self::Empty | Self::Gammaloop | Self::FullVector { .. } => None,
        }
    }

    pub fn moments(&self) -> AccumulatorMomentConfig {
        match self {
            Self::Scalar { moments, .. } | Self::Vector { moments, .. } => *moments,
            Self::Empty | Self::Gammaloop | Self::FullVector { .. } => {
                AccumulatorMomentConfig::default()
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(config) = self.discrete_projections() {
            config.validate()?;
        }
        self.moments().validate()?;
        if let Self::Scalar {
            discrete_projections: Some(config),
            ..
        } = self
            && !config.streams.is_empty()
        {
            return Err(
                "discrete_projections.streams is only valid for vector accumulators".to_string(),
            );
        }
        if let Self::Vector {
            components,
            training_projection,
            discrete_projections,
            ..
        } = self
        {
            validate_vector_accumulator(components, training_projection)?;
            if let Some(config) = discrete_projections {
                validate_discrete_projection_streams(components, config)?;
            }
        }
        if let Self::FullVector { components } = self
            && components.is_empty()
        {
            return Err("full_vector accumulator components must not be empty".to_string());
        }
        Ok(())
    }

    pub fn semantic_kind(&self) -> crate::evaluation::SemanticAccumulatorKind {
        match self {
            Self::Empty | Self::Scalar { .. } => crate::evaluation::SemanticAccumulatorKind::Scalar,
            Self::Vector { .. } => crate::evaluation::SemanticAccumulatorKind::Vector,
            Self::FullVector { components } if components == &["value".to_string()] => {
                crate::evaluation::SemanticAccumulatorKind::Scalar
            }
            Self::FullVector { .. } => crate::evaluation::SemanticAccumulatorKind::Vector,
            Self::Gammaloop => crate::evaluation::SemanticAccumulatorKind::Scalar,
        }
    }
}

fn validate_discrete_projection_streams(
    components: &[String],
    config: &DiscreteProjectionConfig,
) -> Result<(), String> {
    for stream in &config.streams {
        if stream == "training_projection" || components.iter().any(|component| component == stream)
        {
            continue;
        }
        return Err(format!(
            "discrete_projections.streams references unknown vector stream '{stream}'"
        ));
    }
    Ok(())
}

impl Serialize for AccumulatorConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if !serializer.is_human_readable() {
            return self.to_binary().serialize(serializer);
        }

        #[derive(Serialize)]
        struct Rich<'a> {
            kind: &'static str,
            #[serde(
                rename = "discrete_projections",
                skip_serializing_if = "Option::is_none"
            )]
            discrete_projections: Option<&'a DiscreteProjectionConfig>,
            #[serde(skip_serializing_if = "Option::is_none")]
            components: Option<&'a [String]>,
            #[serde(skip_serializing_if = "Option::is_none")]
            training_projection: Option<&'a TrainingProjection>,
            #[serde(skip_serializing_if = "is_default_moment_config")]
            moments: AccumulatorMomentConfig,
        }

        match self {
            Self::Vector {
                components,
                training_projection,
                discrete_projections,
                moments,
            } => Rich {
                kind: self.kind_str(),
                discrete_projections: discrete_projections.as_ref(),
                components: Some(components.as_slice()),
                training_projection: Some(training_projection),
                moments: *moments,
            }
            .serialize(serializer),
            Self::FullVector { components } => Rich {
                kind: self.kind_str(),
                discrete_projections: None,
                components: Some(components.as_slice()),
                training_projection: None,
                moments: AccumulatorMomentConfig::default(),
            }
            .serialize(serializer),
            Self::Scalar {
                discrete_projections: Some(discrete_projections),
                moments,
            } => Rich {
                kind: self.kind_str(),
                discrete_projections: Some(discrete_projections),
                components: None,
                training_projection: None,
                moments: *moments,
            }
            .serialize(serializer),
            Self::Scalar { moments, .. } if *moments != AccumulatorMomentConfig::default() => {
                Rich {
                    kind: self.kind_str(),
                    discrete_projections: None,
                    components: None,
                    training_projection: None,
                    moments: *moments,
                }
                .serialize(serializer)
            }
            _ => serializer.serialize_str(self.kind_str()),
        }
    }
}

impl<'de> Deserialize<'de> for AccumulatorConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if !deserializer.is_human_readable() {
            let binary = BinaryAccumulatorConfig::deserialize(deserializer)?;
            if binary.discrete_projections.is_some()
                && !Self::kind_accepts_discrete_projections(binary.kind)
            {
                return Err(serde::de::Error::custom(
                    "discrete_projections is only valid for scalar and vector accumulators",
                ));
            }
            return Ok(Self::from_parts(
                binary.kind,
                binary.discrete_projections,
                binary.moments,
                binary.components,
                binary.training_projection.map(TrainingProjection::from),
            ));
        }

        struct AccumulatorConfigVisitor;

        impl<'de> Visitor<'de> for AccumulatorConfigVisitor {
            type Value = AccumulatorConfig;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an accumulator name or accumulator config table")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                accumulator_from_kind_str(
                    value,
                    None,
                    AccumulatorMomentConfig::default(),
                    None,
                    None,
                )
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut kind = None::<String>;
                let mut discrete_projections = None::<DiscreteProjectionConfig>;
                let mut moments = None::<AccumulatorMomentConfig>;
                let mut components = None::<Vec<String>>;
                let mut training_projection = None::<TrainingProjection>;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "kind" => {
                            if kind.is_some() {
                                return Err(serde::de::Error::duplicate_field("kind"));
                            }
                            kind = Some(map.next_value()?);
                        }
                        "discrete_projections" => {
                            if discrete_projections.is_some() {
                                return Err(serde::de::Error::duplicate_field(
                                    "discrete_projections",
                                ));
                            }
                            discrete_projections = Some(map.next_value()?);
                        }
                        "components" => {
                            if components.is_some() {
                                return Err(serde::de::Error::duplicate_field("components"));
                            }
                            components = Some(map.next_value()?);
                        }
                        "moments" => {
                            if moments.is_some() {
                                return Err(serde::de::Error::duplicate_field("moments"));
                            }
                            moments = Some(map.next_value()?);
                        }
                        "training_projection" => {
                            if training_projection.is_some() {
                                return Err(serde::de::Error::duplicate_field(
                                    "training_projection",
                                ));
                            }
                            training_projection = Some(map.next_value()?);
                        }
                        other => {
                            return Err(serde::de::Error::unknown_field(
                                other,
                                &[
                                    "kind",
                                    "discrete_projections",
                                    "moments",
                                    "components",
                                    "training_projection",
                                ],
                            ));
                        }
                    }
                }
                let kind = kind.ok_or_else(|| serde::de::Error::missing_field("kind"))?;
                accumulator_from_kind_str(
                    &kind,
                    discrete_projections,
                    moments.unwrap_or_default(),
                    components,
                    training_projection,
                )
            }
        }

        deserializer.deserialize_any(AccumulatorConfigVisitor)
    }
}

fn accumulator_from_kind_str<E>(
    kind: &str,
    discrete_projections: Option<DiscreteProjectionConfig>,
    moments: AccumulatorMomentConfig,
    components: Option<Vec<String>>,
    training_projection: Option<TrainingProjection>,
) -> Result<AccumulatorConfig, E>
where
    E: serde::de::Error,
{
    match kind {
        "empty" => reject_discrete_projections(AccumulatorConfigKind::Empty, discrete_projections)
            .map(|_| AccumulatorConfig::Empty),
        "scalar" => Ok(AccumulatorConfig::Scalar {
            discrete_projections,
            moments,
        }),
        "vector" => {
            let components = components.unwrap_or_else(|| vec!["value".to_string()]);
            if components.is_empty() {
                return Err(E::custom("vector accumulator components must not be empty"));
            }
            let training_projection = training_projection
                .unwrap_or_else(|| TrainingProjection::component(components[0].clone()));
            validate_vector_accumulator(&components, &training_projection).map_err(E::custom)?;
            Ok(AccumulatorConfig::Vector {
                components,
                training_projection,
                discrete_projections,
                moments,
            })
        }
        "gammaloop" => {
            reject_discrete_projections(AccumulatorConfigKind::Gammaloop, discrete_projections)
                .map(|_| AccumulatorConfig::Gammaloop)
        }
        "full_vector" => {
            reject_discrete_projections(AccumulatorConfigKind::FullVector, discrete_projections)?;
            let components = components.unwrap_or_else(|| vec!["value".to_string()]);
            if components.is_empty() {
                return Err(E::custom(
                    "full_vector accumulator components must not be empty",
                ));
            }
            Ok(AccumulatorConfig::FullVector { components })
        }
        other => Err(serde::de::Error::unknown_variant(
            other,
            &["empty", "scalar", "vector", "gammaloop", "full_vector"],
        )),
    }
}

fn validate_vector_accumulator(
    components: &[String],
    training_projection: &TrainingProjection,
) -> Result<(), String> {
    if components.is_empty() {
        return Err("vector accumulator components must not be empty".to_string());
    }
    if components
        .iter()
        .any(|component| component.trim().is_empty())
    {
        return Err("vector accumulator components must not contain empty names".to_string());
    }
    match training_projection {
        TrainingProjection::Component { name } => {
            if !components.iter().any(|component| component == name) {
                return Err(format!(
                    "vector accumulator training projection references unknown component {name:?}"
                ));
            }
        }
        TrainingProjection::Norm => {}
    }
    Ok(())
}

fn reject_discrete_projections<E>(
    kind: AccumulatorConfigKind,
    discrete_projections: Option<DiscreteProjectionConfig>,
) -> Result<(), E>
where
    E: serde::de::Error,
{
    if discrete_projections.is_some() && !AccumulatorConfig::kind_accepts_discrete_projections(kind)
    {
        Err(E::custom(
            "discrete_projections is only valid for scalar and vector accumulators",
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

impl EvaluatorConfig {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SamplerAggregatorConfig {
    NaiveMonteCarlo {
        #[serde(flatten)]
        params: NaiveMonteCarloSamplerParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materializer: Option<MaterializerConfig>,
    },
    RasterPlane {
        #[serde(flatten)]
        params: RasterPlaneSamplerParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materializer: Option<MaterializerConfig>,
    },
    RasterLine {
        #[serde(flatten)]
        params: RasterLineSamplerParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materializer: Option<MaterializerConfig>,
    },
    PdfAdaptationRasterPlane {
        #[serde(flatten)]
        params: RasterPlaneSamplerParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materializer: Option<MaterializerConfig>,
    },
    PdfAdaptationRasterLine {
        #[serde(flatten)]
        params: RasterLineSamplerParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materializer: Option<MaterializerConfig>,
    },
    HavanaTraining {
        #[serde(flatten)]
        params: HavanaSamplerParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materializer: Option<MaterializerConfig>,
    },
    HavanaInference {
        #[serde(flatten)]
        params: HavanaInferenceSamplerParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materializer: Option<MaterializerConfig>,
    },
    ProcessSampler {
        #[serde(flatten)]
        params: ProcessSamplerParams,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        materializer: Option<MaterializerConfig>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaterializerConfig {
    ProcessMaterializer {
        #[serde(flatten)]
        params: ProcessMaterializerParams,
    },
}

#[cfg(test)]
mod tests {
    use super::{AccumulatorConfig, AccumulatorMomentConfig};

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

        let AccumulatorConfig::Scalar {
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
        let err = toml::from_str::<AccumulatorConfig>(
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

        assert!(err.to_string().contains("max_total_bins"));
    }

    #[test]
    fn accumulator_config_parses_moment_config() {
        let config: AccumulatorConfig = toml::from_str(
            r#"
kind = "scalar"
moments = { max_order = 4 }
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
moments = { max_order = 4 }
"#,
        )
        .expect("vector accumulator config");

        assert_eq!(config.moments(), AccumulatorMomentConfig::MaxOrder4);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    ProcessBatchTransform {
        #[serde(flatten)]
        params: ProcessBatchTransformParams,
    },
}
