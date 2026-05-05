mod complex;
mod discrete_bins;
mod empty;
mod full;
mod gammaloop;
mod scalar;

use crate::core::{AccumulatorConfig, EngineError, RunSpec};
use crate::evaluation::batch::Point;
use num::complex::Complex64;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;

pub use self::complex::ComplexAccumulatorState;
pub use self::discrete_bins::{
    ComplexDiscreteBinStats, ComplexDiscreteProjection, ScalarDiscreteBinStats, discrete_bin_key,
};
pub use self::empty::EmptyAccumulatorState;
pub use self::full::{
    ComplexValue, FullAccumulatorProgress, FullComplexAccumulatorState, FullScalarAccumulatorState,
};
pub use self::gammaloop::{
    GammaLoopAccumulatorDigest, GammaLoopAccumulatorState, GammaLoopDiagnostics,
};
pub use self::scalar::ScalarAccumulatorState;

/// Accumulator payloads are persisted and served through JSON-facing APIs.
/// Implementations must therefore keep their serialized state JSON-safe and
/// handle non-finite floating-point contributions explicitly instead of
/// letting serialization degrade them into `null`.
pub trait IngestScalar {
    fn ingest_scalar(&mut self, value: f64, point: &Point);
}

pub trait IngestComplex {
    fn ingest_complex(&mut self, value: Complex64, point: &Point);
}

pub trait Accumulator: Clone + Serialize + DeserializeOwned {
    type Persistent: Clone + Serialize + DeserializeOwned;
    type Digest: Clone + Serialize + DeserializeOwned;

    fn sample_count(&self) -> i64;
    fn merge(&mut self, other: Self);
    fn get_persistent(&self) -> Self::Persistent;
    fn get_digest(&self, _run_spec: &RunSpec) -> Result<Self::Digest, EngineError>
    where
        Self: Into<Self::Digest>,
    {
        Ok(self.clone().into())
    }

    fn to_persistent_json(&self) -> Result<JsonValue, EngineError> {
        serde_json::to_value(self.get_persistent()).map_err(|err| {
            EngineError::build(format!(
                "failed to serialize persistent accumulator payload: {err}"
            ))
        })
    }

    fn to_digest_json(&self, run_spec: &RunSpec) -> Result<JsonValue, EngineError>
    where
        Self: Into<Self::Digest>,
    {
        serde_json::to_value(self.get_digest(run_spec)?).map_err(|err| {
            EngineError::build(format!(
                "failed to serialize accumulator digest payload: {err}"
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccumulatorState {
    Empty(EmptyAccumulatorState),
    Scalar(ScalarAccumulatorState),
    Complex(ComplexAccumulatorState),
    Gammaloop(GammaLoopAccumulatorState),
    FullScalar(FullScalarAccumulatorState),
    FullComplex(FullComplexAccumulatorState),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAccumulatorKind {
    #[default]
    Scalar,
    Complex,
}

impl SemanticAccumulatorKind {
    pub fn aggregate_accumulator_config(self) -> AccumulatorConfig {
        match self {
            Self::Scalar => AccumulatorConfig::scalar(),
            Self::Complex => AccumulatorConfig::complex(),
        }
    }

    pub fn full_accumulator_config(self) -> AccumulatorConfig {
        match self {
            Self::Scalar => AccumulatorConfig::FullScalar,
            Self::Complex => AccumulatorConfig::FullComplex,
        }
    }
}

impl AccumulatorState {
    pub fn from_aggregate_persistent_json(
        kind: SemanticAccumulatorKind,
        value: &JsonValue,
    ) -> Result<Self, EngineError> {
        match kind {
            SemanticAccumulatorKind::Scalar => serde_json::from_value(value.clone())
                .map(Self::Scalar)
                .map_err(|err| {
                    EngineError::build(format!(
                        "invalid scalar persistent accumulator payload: {err}"
                    ))
                }),
            SemanticAccumulatorKind::Complex => serde_json::from_value(value.clone())
                .map(Self::Complex)
                .map_err(|err| {
                    EngineError::build(format!(
                        "invalid complex persistent accumulator payload: {err}"
                    ))
                }),
        }
    }

    pub fn from_gammaloop_persistent_json(value: &JsonValue) -> Result<Self, EngineError> {
        serde_json::from_value(value.clone())
            .map(Self::Gammaloop)
            .map_err(|err| {
                EngineError::build(format!(
                    "invalid gammaloop persistent accumulator payload: {err}"
                ))
            })
    }

    pub fn from_config(config: &AccumulatorConfig) -> Self {
        match config {
            AccumulatorConfig::Empty => Self::empty(),
            AccumulatorConfig::Scalar {
                discrete_histograms,
            } => Self::Scalar(ScalarAccumulatorState::from_config(
                discrete_histograms.clone(),
            )),
            AccumulatorConfig::Complex {
                discrete_histograms,
            } => Self::Complex(ComplexAccumulatorState::from_config(
                discrete_histograms.clone(),
            )),
            AccumulatorConfig::Gammaloop => Self::empty_gammaloop(),
            AccumulatorConfig::FullScalar => Self::empty_full_scalar(),
            AccumulatorConfig::FullComplex => Self::empty_full_complex(),
        }
    }

    pub fn empty() -> Self {
        Self::Empty(EmptyAccumulatorState::default())
    }

    pub fn empty_scalar() -> Self {
        Self::Scalar(ScalarAccumulatorState::default())
    }

    pub fn empty_complex() -> Self {
        Self::Complex(ComplexAccumulatorState::default())
    }

    pub fn empty_gammaloop() -> Self {
        Self::Gammaloop(GammaLoopAccumulatorState::default())
    }

    pub fn empty_full_scalar() -> Self {
        Self::FullScalar(FullScalarAccumulatorState::default())
    }

    pub fn empty_full_complex() -> Self {
        Self::FullComplex(FullComplexAccumulatorState::default())
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Empty(_) => "empty",
            Self::Scalar(_) => "scalar",
            Self::Complex(_) => "complex",
            Self::Gammaloop(_) => "gammaloop",
            Self::FullScalar(_) => "full_scalar",
            Self::FullComplex(_) => "full_complex",
        }
    }

    pub fn config(&self) -> AccumulatorConfig {
        match self {
            Self::Empty(_) => AccumulatorConfig::Empty,
            Self::Scalar(state) => AccumulatorConfig::Scalar {
                discrete_histograms: state.discrete_histograms.clone(),
            },
            Self::Complex(state) => AccumulatorConfig::Complex {
                discrete_histograms: state.discrete_histograms.clone(),
            },
            Self::Gammaloop(_) => AccumulatorConfig::Gammaloop,
            Self::FullScalar(_) => AccumulatorConfig::FullScalar,
            Self::FullComplex(_) => AccumulatorConfig::FullComplex,
        }
    }

    pub fn merge(&mut self, other: Self) -> Result<(), EngineError> {
        match (self, other) {
            (Self::Empty(left), Self::Empty(right)) => {
                Accumulator::merge(left, right);
                Ok(())
            }
            (Self::Scalar(left), Self::Scalar(right)) => {
                Accumulator::merge(left, right);
                Ok(())
            }
            (Self::Complex(left), Self::Complex(right)) => {
                Accumulator::merge(left, right);
                Ok(())
            }
            (Self::Gammaloop(left), Self::Gammaloop(right)) => left.merge_in_place(right),
            (Self::FullScalar(left), Self::FullScalar(right)) => {
                Accumulator::merge(left, right);
                Ok(())
            }
            (Self::FullComplex(left), Self::FullComplex(right)) => {
                Accumulator::merge(left, right);
                Ok(())
            }
            (left, right) => Err(EngineError::engine(format!(
                "cannot merge {} accumulator with {} accumulator",
                left.kind_str(),
                right.kind_str(),
            ))),
        }
    }

    pub fn sample_count(&self) -> i64 {
        match self {
            Self::Empty(accumulator) => accumulator.sample_count(),
            Self::Scalar(accumulator) => accumulator.sample_count(),
            Self::Complex(accumulator) => accumulator.sample_count(),
            Self::Gammaloop(accumulator) => accumulator.sample_count(),
            Self::FullScalar(accumulator) => accumulator.sample_count(),
            Self::FullComplex(accumulator) => accumulator.sample_count(),
        }
    }

    pub fn abs_signal_to_noise(&self) -> f64 {
        match self {
            Self::Empty(_) => 0.0,
            Self::Scalar(accumulator) => accumulator.signal_to_noise(),
            Self::Complex(accumulator) => accumulator.signal_to_noise(),
            Self::Gammaloop(accumulator) => accumulator.signal_to_noise(),
            Self::FullScalar(_) | Self::FullComplex(_) => 0.0,
        }
    }

    pub fn to_json(&self) -> Result<JsonValue, EngineError> {
        serde_json::to_value(self)
            .map_err(|err| EngineError::build(format!("failed to serialize accumulator: {err}")))
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, EngineError> {
        serde_json::from_value(value.clone())
            .map_err(|err| EngineError::build(format!("invalid accumulator payload: {err}")))
    }

    pub fn to_persistent_json(&self) -> Result<JsonValue, EngineError> {
        match self {
            Self::Empty(accumulator) => accumulator.to_persistent_json(),
            Self::Scalar(accumulator) => accumulator.to_persistent_json(),
            Self::Complex(accumulator) => accumulator.to_persistent_json(),
            Self::Gammaloop(accumulator) => accumulator.to_persistent_json(),
            Self::FullScalar(accumulator) => accumulator.to_persistent_json(),
            Self::FullComplex(accumulator) => accumulator.to_persistent_json(),
        }
    }

    pub fn to_digest_json(&self, run_spec: &RunSpec) -> Result<JsonValue, EngineError> {
        match self {
            Self::Empty(accumulator) => accumulator.to_digest_json(run_spec),
            Self::Scalar(accumulator) => accumulator.to_digest_json(run_spec),
            Self::Complex(accumulator) => accumulator.to_digest_json(run_spec),
            Self::Gammaloop(accumulator) => accumulator.to_digest_json(run_spec),
            Self::FullScalar(accumulator) => accumulator.to_digest_json(run_spec),
            Self::FullComplex(accumulator) => accumulator.to_digest_json(run_spec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AccumulatorState, ScalarAccumulatorState};
    use crate::core::{
        AccumulatorConfig, DiscreteHistogramConfig, DiscreteHistogramNormalization,
        NamedDiscreteHistogram,
    };

    #[test]
    fn persistent_json_roundtrips_without_enum_tag() {
        let snapshot = AccumulatorState::Scalar(ScalarAccumulatorState {
            count: 2,
            sum_weighted_value: 3.0,
            sum_abs: 4.0,
            sum_sq: 5.0,
            nan_count: 0,
            ..Default::default()
        })
        .to_persistent_json()
        .expect("persistent snapshot");

        assert_eq!(snapshot.get("kind"), None);
    }

    #[test]
    fn scalar_state_config_preserves_discrete_histograms() {
        let config = AccumulatorConfig::Scalar {
            discrete_histograms: Some(DiscreteHistogramConfig {
                max_total_bins: Some(16),
                normalization: DiscreteHistogramNormalization::Contribution,
                items: vec![NamedDiscreteHistogram {
                    name: "spin".to_string(),
                    hist_dims: vec![0],
                    fixed_dims: Default::default(),
                }],
            }),
        };

        let state = AccumulatorState::from_config(&config);

        assert_eq!(state.config(), config);
    }
}
