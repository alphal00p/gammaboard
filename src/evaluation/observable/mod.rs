mod complex;
mod full;
mod gammaloop;
mod scalar;

use crate::core::{EngineError, ObservableConfig, RunSpec};
use num::complex::Complex64;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;

pub use self::complex::ComplexObservableState;
pub use self::full::{
    ComplexValue, FullComplexObservableState, FullObservableProgress, FullScalarObservableState,
};
pub use self::gammaloop::{GammaLoopObservableDigest, GammaLoopObservableState};
pub use self::scalar::ScalarObservableState;

pub trait IngestScalar {
    fn ingest_scalar(&mut self, value: f64, weight: f64);
}

pub trait IngestComplex {
    fn ingest_complex(&mut self, value: Complex64, weight: f64);
}

pub trait Observable: Clone + Serialize + DeserializeOwned {
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
                "failed to serialize persistent observable payload: {err}"
            ))
        })
    }

    fn to_digest_json(&self, run_spec: &RunSpec) -> Result<JsonValue, EngineError>
    where
        Self: Into<Self::Digest>,
    {
        serde_json::to_value(self.get_digest(run_spec)?).map_err(|err| {
            EngineError::build(format!(
                "failed to serialize observable digest payload: {err}"
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservableState {
    Scalar(ScalarObservableState),
    Complex(ComplexObservableState),
    Gammaloop(GammaLoopObservableState),
    FullScalar(FullScalarObservableState),
    FullComplex(FullComplexObservableState),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SemanticObservableKind {
    #[default]
    Scalar,
    Complex,
}

impl SemanticObservableKind {
    pub fn aggregate_observable_config(self) -> ObservableConfig {
        match self {
            Self::Scalar => ObservableConfig::Scalar,
            Self::Complex => ObservableConfig::Complex,
        }
    }

    pub fn full_observable_config(self) -> ObservableConfig {
        match self {
            Self::Scalar => ObservableConfig::FullScalar,
            Self::Complex => ObservableConfig::FullComplex,
        }
    }
}

impl ObservableState {
    pub fn from_aggregate_persistent_json(
        kind: SemanticObservableKind,
        value: &JsonValue,
    ) -> Result<Self, EngineError> {
        match kind {
            SemanticObservableKind::Scalar => serde_json::from_value(value.clone())
                .map(Self::Scalar)
                .map_err(|err| {
                    EngineError::build(format!(
                        "invalid scalar persistent observable payload: {err}"
                    ))
                }),
            SemanticObservableKind::Complex => serde_json::from_value(value.clone())
                .map(Self::Complex)
                .map_err(|err| {
                    EngineError::build(format!(
                        "invalid complex persistent observable payload: {err}"
                    ))
                }),
        }
    }

    pub fn from_gammaloop_persistent_json(value: &JsonValue) -> Result<Self, EngineError> {
        serde_json::from_value(value.clone())
            .map(|bundle| Self::Gammaloop(GammaLoopObservableState { bundle }))
            .map_err(|err| {
                EngineError::build(format!(
                    "invalid gammaloop persistent observable payload: {err}"
                ))
            })
    }

    pub fn from_config(config: &ObservableConfig) -> Self {
        match config {
            ObservableConfig::Scalar => Self::empty_scalar(),
            ObservableConfig::Complex => Self::empty_complex(),
            ObservableConfig::Gammaloop => Self::empty_gammaloop(),
            ObservableConfig::FullScalar => Self::empty_full_scalar(),
            ObservableConfig::FullComplex => Self::empty_full_complex(),
        }
    }

    pub fn empty_scalar() -> Self {
        Self::Scalar(ScalarObservableState::default())
    }

    pub fn empty_complex() -> Self {
        Self::Complex(ComplexObservableState::default())
    }

    pub fn empty_gammaloop() -> Self {
        Self::Gammaloop(GammaLoopObservableState::default())
    }

    pub fn empty_full_scalar() -> Self {
        Self::FullScalar(FullScalarObservableState::default())
    }

    pub fn empty_full_complex() -> Self {
        Self::FullComplex(FullComplexObservableState::default())
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Scalar(_) => "scalar",
            Self::Complex(_) => "complex",
            Self::Gammaloop(_) => "gammaloop",
            Self::FullScalar(_) => "full_scalar",
            Self::FullComplex(_) => "full_complex",
        }
    }

    pub fn config(&self) -> ObservableConfig {
        match self {
            Self::Scalar(_) => ObservableConfig::Scalar,
            Self::Complex(_) => ObservableConfig::Complex,
            Self::Gammaloop(_) => ObservableConfig::Gammaloop,
            Self::FullScalar(_) => ObservableConfig::FullScalar,
            Self::FullComplex(_) => ObservableConfig::FullComplex,
        }
    }

    pub fn merge(&mut self, other: Self) -> Result<(), EngineError> {
        match (self, other) {
            (Self::Scalar(left), Self::Scalar(right)) => {
                Observable::merge(left, right);
                Ok(())
            }
            (Self::Complex(left), Self::Complex(right)) => {
                Observable::merge(left, right);
                Ok(())
            }
            (Self::Gammaloop(left), Self::Gammaloop(right)) => left.merge_in_place(right),
            (Self::FullScalar(left), Self::FullScalar(right)) => {
                Observable::merge(left, right);
                Ok(())
            }
            (Self::FullComplex(left), Self::FullComplex(right)) => {
                Observable::merge(left, right);
                Ok(())
            }
            (left, right) => Err(EngineError::engine(format!(
                "cannot merge {} observable with {} observable",
                left.kind_str(),
                right.kind_str(),
            ))),
        }
    }

    pub fn sample_count(&self) -> i64 {
        match self {
            Self::Scalar(observable) => observable.sample_count(),
            Self::Complex(observable) => observable.sample_count(),
            Self::Gammaloop(observable) => observable.sample_count(),
            Self::FullScalar(observable) => observable.sample_count(),
            Self::FullComplex(observable) => observable.sample_count(),
        }
    }

    pub fn abs_signal_to_noise(&self) -> f64 {
        match self {
            Self::Scalar(observable) => observable.signal_to_noise(),
            Self::Complex(observable) => observable.signal_to_noise(),
            Self::Gammaloop(observable) => observable.signal_to_noise(),
            Self::FullScalar(_) | Self::FullComplex(_) => 0.0,
        }
    }

    pub fn to_json(&self) -> Result<JsonValue, EngineError> {
        serde_json::to_value(self)
            .map_err(|err| EngineError::build(format!("failed to serialize observable: {err}")))
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, EngineError> {
        serde_json::from_value(value.clone())
            .map_err(|err| EngineError::build(format!("invalid observable payload: {err}")))
    }

    pub fn to_persistent_json(&self) -> Result<JsonValue, EngineError> {
        match self {
            Self::Scalar(observable) => observable.to_persistent_json(),
            Self::Complex(observable) => observable.to_persistent_json(),
            Self::Gammaloop(observable) => observable.to_persistent_json(),
            Self::FullScalar(observable) => observable.to_persistent_json(),
            Self::FullComplex(observable) => observable.to_persistent_json(),
        }
    }

    pub fn to_digest_json(&self, run_spec: &RunSpec) -> Result<JsonValue, EngineError> {
        match self {
            Self::Scalar(observable) => observable.to_digest_json(run_spec),
            Self::Complex(observable) => observable.to_digest_json(run_spec),
            Self::Gammaloop(observable) => observable.to_digest_json(run_spec),
            Self::FullScalar(observable) => observable.to_digest_json(run_spec),
            Self::FullComplex(observable) => observable.to_digest_json(run_spec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ObservableState, ScalarObservableState};

    #[test]
    fn persistent_json_roundtrips_without_enum_tag() {
        let snapshot = ObservableState::Scalar(ScalarObservableState {
            count: 2,
            sum_weighted_value: 3.0,
            sum_abs: 4.0,
            sum_sq: 5.0,
        })
        .to_persistent_json()
        .expect("persistent snapshot");

        assert_eq!(snapshot.get("kind"), None);
    }
}
