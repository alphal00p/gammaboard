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
pub use self::metrics::{
    AccumulatorMetricValue, extract_accumulator_metric, extract_accumulator_metric_with_runtime,
    relative_error,
};
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
    Vector(VectorAccumulatorState),
    Gammaloop(GammaLoopAccumulatorState),
    FullVector(FullVectorAccumulatorState),
}

pub const SCALAR_COMPONENT_NAME: &str = "value";

fn scalar_value_vector(
    discrete_projections: Option<crate::core::DiscreteProjectionConfig>,
    moments: crate::core::AccumulatorMomentConfig,
) -> VectorAccumulatorState {
    VectorAccumulatorState::from_config(
        vec![SCALAR_COMPONENT_NAME.to_string()],
        crate::core::TrainingProjection::component(SCALAR_COMPONENT_NAME),
        discrete_projections,
        moments,
    )
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
        _kind: SemanticAccumulatorKind,
        value: &JsonValue,
    ) -> Result<Self, EngineError> {
        // Scalar accumulators are stored as one-component vectors, so both
        // semantic kinds deserialize into a vector state.
        Self::from_vector_persistent_json(value)
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
            } => Self::Vector(scalar_value_vector(discrete_projections.clone(), *moments)),
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
        Self::empty_vector()
    }

    pub fn empty_vector() -> Self {
        Self::Vector(scalar_value_vector(None, Default::default()))
    }

    pub fn empty_gammaloop() -> Self {
        Self::Gammaloop(GammaLoopAccumulatorState::default())
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Empty(_) => "empty",
            Self::Vector(_) => "vector",
            Self::Gammaloop(_) => "gammaloop",
            Self::FullVector(_) => "full_vector",
        }
    }

    pub fn config(&self) -> AccumulatorConfig {
        match self {
            Self::Empty(_) => AccumulatorConfig::Empty,
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
            Self::Vector(accumulator) => accumulator.sample_count(),
            Self::Gammaloop(accumulator) => accumulator.sample_count(),
            Self::FullVector(accumulator) => accumulator.sample_count(),
        }
    }

    pub fn abs_signal_to_noise(&self) -> f64 {
        match self {
            Self::Empty(_) => 0.0,
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
            Self::Vector(accumulator) => accumulator.to_persistent_json(),
            Self::Gammaloop(accumulator) => accumulator.to_persistent_json(),
            Self::FullVector(accumulator) => accumulator.to_persistent_json(),
        }
    }

    pub fn to_digest_json(&self, run_spec: &RunSpec) -> Result<JsonValue, EngineError> {
        match self {
            Self::Empty(accumulator) => accumulator.to_digest_json(run_spec),
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
        TrainingProjection,
    };
    use crate::evaluation::Point;
    use crate::{NamedScalarAccumulator, VectorAccumulatorState};

    #[test]
    fn persistent_json_roundtrips_without_enum_tag() {
        let snapshot = AccumulatorState::Vector(VectorAccumulatorState {
            components: vec![NamedScalarAccumulator {
                name: "value".to_string(),
                state: ScalarAccumulatorState {
                    count: 2,
                    sum_weighted_value: 3.0,
                    sum_abs: 4.0,
                    sum_sq: 5.0,
                    nan_count: 0,
                    ..Default::default()
                },
            }],
            projection_spec: TrainingProjection::component("value"),
            projection: NamedScalarAccumulator {
                name: "value".to_string(),
                state: ScalarAccumulatorState {
                    count: 2,
                    sum_weighted_value: 3.0,
                    sum_abs: 4.0,
                    sum_sq: 5.0,
                    nan_count: 0,
                    ..Default::default()
                },
            },
        })
        .to_persistent_json()
        .expect("persistent snapshot");

        assert_eq!(snapshot.get("kind"), None);
    }

    #[test]
    fn scalar_state_config_preserves_discrete_projections() {
        let config = AccumulatorConfig::Scalar {
            discrete_projections: Some(DiscreteProjectionConfig {
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

        let equivalent_vector_config = AccumulatorConfig::Vector {
            components: vec!["value".to_string()],
            training_projection: TrainingProjection::component("value"),
            discrete_projections: config.discrete_projections().cloned(),
            moments: Default::default(),
        };

        let state = AccumulatorState::from_config(&config);

        assert_eq!(state.config(), equivalent_vector_config);
    }

    #[test]
    fn vector_metric_extraction_reports_component_variance_uncertainty_with_fourth_moment() {
        let config = AccumulatorConfig::Vector {
            components: vec!["real".to_string(), "imag".to_string()],
            training_projection: TrainingProjection::component("real"),
            discrete_projections: None,
            moments: crate::core::AccumulatorMomentConfig::MaxOrder4,
        };
        let AccumulatorState::Vector(mut state) = AccumulatorState::from_config(&config) else {
            panic!("expected vector accumulator");
        };

        let point = Point::new(vec![], vec![], 1.0);
        for values in [[1.0, 0.5], [2.0, 1.0], [4.0, 2.0], [8.0, 4.0]] {
            state.ingest_vector(&values, &point).expect("vector ingest");
        }

        let metric = extract_accumulator_metric(
            &AccumulatorState::Vector(state),
            &AccumulatorMetricSelector {
                name: AccumulatorMetricName::Variance,
                component: Some("real".to_string()),
            },
        )
        .expect("metric extraction")
        .expect("variance metric");

        assert_eq!(metric.component.as_deref(), Some("real"));
        assert!(metric.value > 0.0);
        assert!(metric.uncertainty.is_some_and(|value| value > 0.0));
        assert_eq!(metric.sample_count, 4);
    }
}
