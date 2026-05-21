mod discrete_bins;
mod empty;
mod full;
#[cfg(feature = "gammaloop")]
mod gammaloop;
#[cfg(not(feature = "gammaloop"))]
#[path = "gammaloop_disabled.rs"]
mod gammaloop;
mod metrics;
mod scalar;
mod vector;

use crate::core::{AccumulatorConfig, EngineError, RunSpec};
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value as JsonValue;

pub use self::discrete_bins::{DiscreteProjectionBinState, discrete_bin_key};
pub use self::empty::EmptyAccumulatorState;
pub use self::full::{FullAccumulatorProgress, FullVectorAccumulatorState};
pub use self::gammaloop::{
    GammaLoopAccumulatorDigest, GammaLoopAccumulatorState, GammaLoopDiagnostics,
};
pub use self::metrics::{AccumulatorMetricValue, extract_accumulator_metric, relative_error};
pub use self::scalar::ScalarAccumulatorState;
pub use self::vector::{NamedScalarAccumulator, VectorAccumulatorState};

/// Accumulator payloads are persisted and served through JSON-facing APIs.
/// Implementations must therefore keep their serialized state JSON-safe and
/// handle non-finite floating-point contributions explicitly instead of
/// letting serialization degrade them into `null`.
pub trait IngestScalar {
    fn ingest_scalar(&mut self, value: f64, point: &Point);
}

pub trait IngestVector {
    fn ingest_vector(&mut self, values: &[f64], point: &Point) -> Result<f64, String>;
}

impl IngestVector for VectorAccumulatorState {
    fn ingest_vector(&mut self, values: &[f64], point: &Point) -> Result<f64, String> {
        VectorAccumulatorState::ingest_vector(self, values, point)
    }
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
    Vector(VectorAccumulatorState),
    Gammaloop(GammaLoopAccumulatorState),
    FullVector(FullVectorAccumulatorState),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAccumulatorKind {
    #[default]
    Scalar,
    Vector,
}

impl SemanticAccumulatorKind {
    pub fn aggregate_accumulator_config(self) -> AccumulatorConfig {
        match self {
            Self::Scalar => AccumulatorConfig::scalar(),
            Self::Vector => AccumulatorConfig::vector(
                vec!["real".to_string(), "imag".to_string()],
                crate::core::TrainingProjection::Norm,
            ),
        }
    }

    pub fn full_accumulator_config(self) -> AccumulatorConfig {
        match self {
            Self::Scalar => AccumulatorConfig::FullVector {
                components: vec!["value".to_string()],
            },
            Self::Vector => AccumulatorConfig::FullVector {
                components: vec!["real".to_string(), "imag".to_string()],
            },
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
            SemanticAccumulatorKind::Vector => serde_json::from_value(value.clone())
                .map(Self::Vector)
                .map_err(|err| {
                    EngineError::build(format!(
                        "invalid vector persistent accumulator payload: {err}"
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

    pub fn from_vector_persistent_json(value: &JsonValue) -> Result<Self, EngineError> {
        serde_json::from_value(value.clone())
            .map(Self::Vector)
            .map_err(|err| {
                EngineError::build(format!(
                    "invalid vector persistent accumulator payload: {err}"
                ))
            })
    }

    pub fn from_config(config: &AccumulatorConfig) -> Self {
        match config {
            AccumulatorConfig::Empty => Self::empty(),
            AccumulatorConfig::Scalar {
                discrete_projections,
                moments,
            } => Self::Scalar(ScalarAccumulatorState::from_config(
                discrete_projections.clone(),
                *moments,
            )),
            AccumulatorConfig::Vector {
                components,
                training_projection,
                discrete_projections,
                moments,
            } => Self::Vector(VectorAccumulatorState::from_config(
                components.clone(),
                training_projection.clone(),
                discrete_projections.clone(),
                *moments,
            )),
            AccumulatorConfig::Gammaloop => Self::empty_gammaloop(),
            AccumulatorConfig::FullVector { components } => Self::FullVector(
                FullVectorAccumulatorState::from_components(components.clone()),
            ),
        }
    }

    pub fn empty() -> Self {
        Self::Empty(EmptyAccumulatorState::default())
    }

    pub fn empty_scalar() -> Self {
        Self::Scalar(ScalarAccumulatorState::default())
    }

    pub fn empty_vector() -> Self {
        Self::Vector(VectorAccumulatorState::from_config(
            vec!["value".to_string()],
            crate::core::TrainingProjection::component("value"),
            None,
            Default::default(),
        ))
    }

    pub fn empty_gammaloop() -> Self {
        Self::Gammaloop(GammaLoopAccumulatorState::default())
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Empty(_) => "empty",
            Self::Scalar(_) => "scalar",
            Self::Vector(_) => "vector",
            Self::Gammaloop(_) => "gammaloop",
            Self::FullVector(_) => "full_vector",
        }
    }

    pub fn config(&self) -> AccumulatorConfig {
        match self {
            Self::Empty(_) => AccumulatorConfig::Empty,
            Self::Scalar(state) => AccumulatorConfig::Scalar {
                discrete_projections: state.discrete_projections.clone(),
                moments: state.moments,
            },
            Self::Vector(state) => AccumulatorConfig::Vector {
                components: state
                    .components
                    .iter()
                    .map(|component| component.name.clone())
                    .collect(),
                training_projection: state.projection_spec.clone(),
                discrete_projections: state
                    .components
                    .first()
                    .and_then(|component| component.state.discrete_projections.clone()),
                moments: state.projection.state.moments,
            },
            Self::Gammaloop(_) => AccumulatorConfig::Gammaloop,
            Self::FullVector(state) => AccumulatorConfig::FullVector {
                components: state.components.clone(),
            },
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
            (Self::Vector(left), Self::Vector(right)) => {
                Accumulator::merge(left, right);
                Ok(())
            }
            (Self::Gammaloop(left), Self::Gammaloop(right)) => left.merge_in_place(right),
            (Self::FullVector(left), Self::FullVector(right)) => {
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
            Self::Vector(accumulator) => accumulator.sample_count(),
            Self::Gammaloop(accumulator) => accumulator.sample_count(),
            Self::FullVector(accumulator) => accumulator.sample_count(),
        }
    }

    pub fn abs_signal_to_noise(&self) -> f64 {
        match self {
            Self::Empty(_) => 0.0,
            Self::Scalar(accumulator) => accumulator.signal_to_noise(),
            Self::Vector(accumulator) => accumulator.signal_to_noise(),
            Self::Gammaloop(accumulator) => accumulator.signal_to_noise(),
            Self::FullVector(_) => 0.0,
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
            Self::Vector(accumulator) => accumulator.to_persistent_json(),
            Self::Gammaloop(accumulator) => accumulator.to_persistent_json(),
            Self::FullVector(accumulator) => accumulator.to_persistent_json(),
        }
    }

    pub fn to_digest_json(&self, run_spec: &RunSpec) -> Result<JsonValue, EngineError> {
        match self {
            Self::Empty(accumulator) => accumulator.to_digest_json(run_spec),
            Self::Scalar(accumulator) => accumulator.to_digest_json(run_spec),
            Self::Vector(accumulator) => accumulator.to_digest_json(run_spec),
            Self::Gammaloop(accumulator) => accumulator.to_digest_json(run_spec),
            Self::FullVector(accumulator) => accumulator.to_digest_json(run_spec),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AccumulatorState, ScalarAccumulatorState, extract_accumulator_metric};
    use crate::core::{
        AccumulatorConfig, AccumulatorMetricName, AccumulatorMetricSelector,
        DiscreteProjectionConfig, DiscreteProjectionNormalization, NamedDiscreteProjection,
    };
    use crate::evaluation::Point;

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
    fn scalar_state_config_preserves_discrete_projections() {
        let config = AccumulatorConfig::Scalar {
            discrete_projections: Some(DiscreteProjectionConfig {
                max_total_bins: Some(16),
                normalization: DiscreteProjectionNormalization::Contribution,
                streams: Vec::new(),
                items: vec![NamedDiscreteProjection {
                    name: "spin".to_string(),
                    dims: vec![0],
                    fixed_dims: Default::default(),
                }],
            }),
            moments: Default::default(),
        };

        let state = AccumulatorState::from_config(&config);

        assert_eq!(state.config(), config);
    }

    #[test]
    fn scalar_metric_extraction_reports_variance_uncertainty_with_fourth_moment() {
        let config = AccumulatorConfig::Scalar {
            discrete_projections: None,
            moments: crate::core::AccumulatorMomentConfig::MaxOrder4,
        };
        let AccumulatorState::Scalar(mut state) = AccumulatorState::from_config(&config) else {
            panic!("expected scalar accumulator");
        };

        let point = Point::new(vec![], vec![], 1.0);
        for value in [1.0, 2.0, 4.0, 8.0] {
            state.add_sample(value, &point);
        }

        let metric = extract_accumulator_metric(
            &AccumulatorState::Scalar(state),
            &AccumulatorMetricSelector {
                name: AccumulatorMetricName::Variance,
                component: None,
            },
        )
        .expect("metric extraction")
        .expect("variance metric");

        assert!(metric.value > 0.0);
        assert!(metric.uncertainty.is_some_and(|value| value > 0.0));
        assert_eq!(metric.sample_count, 4);
    }
}
