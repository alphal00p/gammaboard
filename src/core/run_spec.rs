use crate::evaluation::{
    GammaLoopParams, ProcessEvaluatorParams, SymbolicaParams, UnitEvaluatorParams,
};
use crate::utils::domain::Domain;

use crate::core::tasks::DiscreteHistogramConfig;
use crate::runners::{EvaluatorRunnerParams, SamplerAggregatorRunnerParams};
use crate::sampling::HavanaInferenceSamplerParams;
use crate::sampling::{
    HavanaSamplerParams, NaiveMonteCarloSamplerParams, ProcessSamplerParams,
    RasterLineSamplerParams, RasterPlaneSamplerParams, SphericalBatchTransformParams,
    UnitBallBatchTransformParams,
};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;

pub type CapabilityRequirements = BTreeMap<String, u64>;

/// Canonical integration parameters payload stored on `runs.integration_params`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationParams {
    pub evaluator: EvaluatorConfig,
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
    pub evaluator: EvaluatorConfig,
    pub evaluator_requirements: CapabilityRequirements,
    pub sampler_requirements: CapabilityRequirements,
    pub evaluator_runner_params: EvaluatorRunnerParams,
    pub sampler_aggregator_runner_params: SamplerAggregatorRunnerParams,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccumulatorConfig {
    Empty,
    Scalar {
        discrete_histograms: Option<DiscreteHistogramConfig>,
    },
    Complex {
        discrete_histograms: Option<DiscreteHistogramConfig>,
    },
    Gammaloop,
    FullScalar,
    FullComplex,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AccumulatorConfigKind {
    Empty,
    Scalar,
    Complex,
    Gammaloop,
    FullScalar,
    FullComplex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BinaryAccumulatorConfig {
    kind: AccumulatorConfigKind,
    discrete_histograms: Option<DiscreteHistogramConfig>,
}

impl AccumulatorConfig {
    pub fn scalar() -> Self {
        Self::Scalar {
            discrete_histograms: None,
        }
    }

    pub fn complex() -> Self {
        Self::Complex {
            discrete_histograms: None,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Scalar { .. } => "scalar",
            Self::Complex { .. } => "complex",
            Self::Gammaloop => "gammaloop",
            Self::FullScalar => "full_scalar",
            Self::FullComplex => "full_complex",
        }
    }

    fn kind(&self) -> AccumulatorConfigKind {
        match self {
            Self::Empty => AccumulatorConfigKind::Empty,
            Self::Scalar { .. } => AccumulatorConfigKind::Scalar,
            Self::Complex { .. } => AccumulatorConfigKind::Complex,
            Self::Gammaloop => AccumulatorConfigKind::Gammaloop,
            Self::FullScalar => AccumulatorConfigKind::FullScalar,
            Self::FullComplex => AccumulatorConfigKind::FullComplex,
        }
    }

    fn to_binary(&self) -> BinaryAccumulatorConfig {
        BinaryAccumulatorConfig {
            kind: self.kind(),
            discrete_histograms: self.discrete_histograms().cloned(),
        }
    }

    fn from_parts(
        kind: AccumulatorConfigKind,
        discrete_histograms: Option<DiscreteHistogramConfig>,
    ) -> Self {
        match kind {
            AccumulatorConfigKind::Empty => Self::Empty,
            AccumulatorConfigKind::Scalar => Self::Scalar {
                discrete_histograms,
            },
            AccumulatorConfigKind::Complex => Self::Complex {
                discrete_histograms,
            },
            AccumulatorConfigKind::Gammaloop => Self::Gammaloop,
            AccumulatorConfigKind::FullScalar => Self::FullScalar,
            AccumulatorConfigKind::FullComplex => Self::FullComplex,
        }
    }

    fn kind_accepts_discrete_histograms(kind: AccumulatorConfigKind) -> bool {
        matches!(
            kind,
            AccumulatorConfigKind::Scalar | AccumulatorConfigKind::Complex
        )
    }

    pub fn discrete_histograms(&self) -> Option<&DiscreteHistogramConfig> {
        match self {
            Self::Scalar {
                discrete_histograms,
            }
            | Self::Complex {
                discrete_histograms,
            } => discrete_histograms.as_ref(),
            Self::Empty | Self::Gammaloop | Self::FullScalar | Self::FullComplex => None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(config) = self.discrete_histograms() {
            config.validate()?;
        }
        Ok(())
    }

    pub fn semantic_kind(&self) -> crate::evaluation::SemanticAccumulatorKind {
        match self {
            Self::Empty | Self::Scalar { .. } | Self::FullScalar => {
                crate::evaluation::SemanticAccumulatorKind::Scalar
            }
            Self::Complex { .. } | Self::FullComplex => {
                crate::evaluation::SemanticAccumulatorKind::Complex
            }
            Self::Gammaloop => crate::evaluation::SemanticAccumulatorKind::Scalar,
        }
    }
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
            #[serde(skip_serializing_if = "Option::is_none")]
            discrete_histograms: Option<&'a DiscreteHistogramConfig>,
        }

        match self {
            Self::Scalar {
                discrete_histograms: Some(discrete_histograms),
            }
            | Self::Complex {
                discrete_histograms: Some(discrete_histograms),
            } => Rich {
                kind: self.kind_str(),
                discrete_histograms: Some(discrete_histograms),
            }
            .serialize(serializer),
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
            if binary.discrete_histograms.is_some()
                && !Self::kind_accepts_discrete_histograms(binary.kind)
            {
                return Err(serde::de::Error::custom(
                    "discrete_histograms is only valid for scalar and complex accumulators",
                ));
            }
            return Ok(Self::from_parts(binary.kind, binary.discrete_histograms));
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
                accumulator_from_kind_str(value, None)
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut kind = None::<String>;
                let mut discrete_histograms = None::<DiscreteHistogramConfig>;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "kind" => {
                            if kind.is_some() {
                                return Err(serde::de::Error::duplicate_field("kind"));
                            }
                            kind = Some(map.next_value()?);
                        }
                        "discrete_histograms" => {
                            if discrete_histograms.is_some() {
                                return Err(serde::de::Error::duplicate_field(
                                    "discrete_histograms",
                                ));
                            }
                            discrete_histograms = Some(map.next_value()?);
                        }
                        other => {
                            return Err(serde::de::Error::unknown_field(
                                other,
                                &["kind", "discrete_histograms"],
                            ));
                        }
                    }
                }
                let kind = kind.ok_or_else(|| serde::de::Error::missing_field("kind"))?;
                accumulator_from_kind_str(&kind, discrete_histograms)
            }
        }

        deserializer.deserialize_any(AccumulatorConfigVisitor)
    }
}

fn accumulator_from_kind_str<E>(
    kind: &str,
    discrete_histograms: Option<DiscreteHistogramConfig>,
) -> Result<AccumulatorConfig, E>
where
    E: serde::de::Error,
{
    match kind {
        "empty" => reject_discrete_histograms(AccumulatorConfigKind::Empty, discrete_histograms)
            .map(|_| AccumulatorConfig::Empty),
        "scalar" => Ok(AccumulatorConfig::Scalar {
            discrete_histograms,
        }),
        "complex" => Ok(AccumulatorConfig::Complex {
            discrete_histograms,
        }),
        "gammaloop" => {
            reject_discrete_histograms(AccumulatorConfigKind::Gammaloop, discrete_histograms)
                .map(|_| AccumulatorConfig::Gammaloop)
        }
        "full_scalar" => {
            reject_discrete_histograms(AccumulatorConfigKind::FullScalar, discrete_histograms)
                .map(|_| AccumulatorConfig::FullScalar)
        }
        "full_complex" => {
            reject_discrete_histograms(AccumulatorConfigKind::FullComplex, discrete_histograms)
                .map(|_| AccumulatorConfig::FullComplex)
        }
        other => Err(serde::de::Error::unknown_variant(
            other,
            &[
                "empty",
                "scalar",
                "complex",
                "gammaloop",
                "full_scalar",
                "full_complex",
            ],
        )),
    }
}

fn reject_discrete_histograms<E>(
    kind: AccumulatorConfigKind,
    discrete_histograms: Option<DiscreteHistogramConfig>,
) -> Result<(), E>
where
    E: serde::de::Error,
{
    if discrete_histograms.is_some() && !AccumulatorConfig::kind_accepts_discrete_histograms(kind) {
        Err(E::custom(
            "discrete_histograms is only valid for scalar and complex accumulators",
        ))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        params: ProcessSamplerParams,
    },
}

#[cfg(test)]
mod tests {
    use super::AccumulatorConfig;

    #[test]
    fn accumulator_config_parses_discrete_histograms() {
        let config: AccumulatorConfig = toml::from_str(
            r#"
kind = "scalar"

[discrete_histograms]
max_total_bins = 16
normalization = "conditional_mean"

[[discrete_histograms.items]]
name = "channel_for_spin_0"
hist_dims = [1]
fixed_dims = { "0" = 0 }
"#,
        )
        .expect("accumulator config");

        let AccumulatorConfig::Scalar {
            discrete_histograms: Some(histograms),
        } = config
        else {
            panic!("expected scalar histogram config");
        };
        assert_eq!(histograms.items[0].fixed_dims.get("0"), Some(&0));
        assert_eq!(
            histograms.normalization,
            crate::core::DiscreteHistogramNormalization::ConditionalMean
        );
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
