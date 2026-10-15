use super::{Accumulator, IngestScalar, ScalarAccumulatorState};
use crate::core::{
    AccumulatorMomentConfig, DiscreteProjectionConfig, EngineError, TrainingProjection,
};
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamedScalarAccumulator {
    pub name: String,
    pub state: ScalarAccumulatorState,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct VectorAccumulatorState {
    pub components: Vec<NamedScalarAccumulator>,
    projection: ProjectionState,
}

/// Component projections reference existing statistics; only a norm needs its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ProjectionState {
    Component { name: String },
    Norm { state: Box<NamedScalarAccumulator> },
}

impl<'de> Deserialize<'de> for VectorAccumulatorState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct State {
            components: Vec<NamedScalarAccumulator>,
            projection: ProjectionState,
        }
        let state = State::deserialize(deserializer)?;
        let state = Self {
            components: state.components,
            projection: state.projection,
        };
        state.validate().map_err(serde::de::Error::custom)?;
        Ok(state)
    }
}

impl VectorAccumulatorState {
    pub fn from_config(
        components: Vec<String>,
        projection_spec: TrainingProjection,
        discrete_projections: Option<DiscreteProjectionConfig>,
        moments: AccumulatorMomentConfig,
    ) -> Self {
        let components = components
            .into_iter()
            .map(|name| NamedScalarAccumulator {
                name,
                state: ScalarAccumulatorState::from_config(discrete_projections.clone(), moments),
            })
            .collect();
        Self {
            components,
            projection: match projection_spec {
                TrainingProjection::Component { name } => ProjectionState::Component { name },
                TrainingProjection::Norm => ProjectionState::Norm {
                    state: Box::new(NamedScalarAccumulator {
                        name: "training_projection".into(),
                        state: ScalarAccumulatorState::from_config(discrete_projections, moments),
                    }),
                },
            },
        }
    }

    pub fn ingest_vector(&mut self, values: &[f64], point: &Point) -> Result<f64, EngineError> {
        self.ingest_vector_with_details(values, point, None)
    }

    pub(crate) fn ingest_vector_with_details(
        &mut self,
        values: &[f64],
        point: &Point,
        evaluation: Option<&crate::evaluation::batch::SampleEvaluation>,
    ) -> Result<f64, EngineError> {
        let projected = self.project(values)?;
        for (component, value) in self.components.iter_mut().zip(values.iter()) {
            component
                .state
                .add_sample_with_details(*value, point, evaluation)
                .map_err(|err| {
                    EngineError::eval(format!("component {:?}: {err}", component.name))
                })?;
        }
        if let ProjectionState::Norm { state } = &mut self.projection {
            state
                .state
                .add_sample_with_details(projected, point, evaluation)
                .map_err(|err| EngineError::eval(format!("training projection: {err}")))?;
        }
        Ok(projected)
    }

    pub fn project(&self, values: &[f64]) -> Result<f64, EngineError> {
        if values.len() != self.components.len() {
            return Err(EngineError::eval(format!(
                "vector observable value count mismatch: expected {}, got {}",
                self.components.len(),
                values.len()
            )));
        }
        match &self.projection {
            ProjectionState::Component { name } => {
                let index = self
                    .components
                    .iter()
                    .position(|component| component.name == *name)
                    .ok_or_else(|| {
                        EngineError::eval(format!(
                            "training projection references unknown component {name:?}"
                        ))
                    })?;
                Ok(values[index])
            }
            ProjectionState::Norm { .. } => Ok(values
                .iter()
                .fold(0.0_f64, |norm, value| norm.hypot(*value))),
        }
    }

    pub fn projection_spec(&self) -> TrainingProjection {
        match &self.projection {
            ProjectionState::Component { name } => TrainingProjection::component(name),
            ProjectionState::Norm { .. } => TrainingProjection::Norm,
        }
    }

    pub fn projection(&self) -> &NamedScalarAccumulator {
        match &self.projection {
            ProjectionState::Component { name } => self
                .component(name)
                .expect("validated projection component"),
            ProjectionState::Norm { state } => state,
        }
    }

    pub fn streams(&self) -> impl Iterator<Item = (&str, &ScalarAccumulatorState)> {
        self.components
            .iter()
            .map(|c| (c.name.as_str(), &c.state))
            .chain(std::iter::once((
                "training_projection",
                &self.projection().state,
            )))
    }

    #[cfg(test)]
    pub(crate) fn projection_mut(&mut self) -> &mut NamedScalarAccumulator {
        match &mut self.projection {
            ProjectionState::Component { name } => self
                .components
                .iter_mut()
                .find(|c| c.name == *name)
                .expect("validated projection component"),
            ProjectionState::Norm { state } => state,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), EngineError> {
        let config = crate::AccumulatorConfig::vector(
            self.components.iter().map(|c| c.name.clone()).collect(),
            self.projection_spec(),
        );
        config.validate().map_err(EngineError::eval)?;
        self.projection().state.validate()?;
        for component in &self.components {
            component.state.validate()?;
            if component.state.count != self.projection().state.count {
                return Err(EngineError::eval(
                    "vector components have inconsistent sample counts",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn validate_merge(&self, other: &Self) -> Result<(), crate::core::EngineError> {
        self.validate()?;
        other.validate()?;
        if self.components.len() != other.components.len()
            || self.projection_spec() != other.projection_spec()
            || self
                .components
                .iter()
                .zip(&other.components)
                .any(|(a, b)| a.name != b.name)
        {
            return Err(crate::core::EngineError::engine(
                "incompatible vector accumulator layout or projection",
            ));
        }
        for (left, right) in self.components.iter().zip(&other.components) {
            left.state.validate_merge(&right.state)?;
        }
        self.projection()
            .state
            .validate_merge(&other.projection().state)
    }

    pub fn primary(&self) -> Option<&NamedScalarAccumulator> {
        self.components.first()
    }

    pub fn component(&self, name: &str) -> Option<&NamedScalarAccumulator> {
        self.components
            .iter()
            .find(|component| component.name == name)
    }

    pub fn rsd(&self) -> f64 {
        self.projection().state.rsd()
    }

    pub fn ess(&self) -> f64 {
        self.projection().state.ess()
    }

    pub fn signal_to_noise(&self) -> f64 {
        self.projection().state.signal_to_noise()
    }
}

impl IngestScalar for VectorAccumulatorState {
    fn ingest_scalar(&mut self, value: f64, point: &Point) -> Result<(), EngineError> {
        self.ingest_vector(&[value], point).map(|_| ())
    }
}

impl Accumulator for VectorAccumulatorState {
    type Persistent = Self;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        self.projection().state.sample_count()
    }

    fn merge(&mut self, other: Self) -> Result<(), crate::core::EngineError> {
        self.validate_merge(&other)?;
        for (left, right) in self.components.iter_mut().zip(other.components) {
            Accumulator::merge(&mut left.state, right.state)?;
        }
        if let (ProjectionState::Norm { state: left }, ProjectionState::Norm { state: right }) =
            (&mut self.projection, other.projection)
        {
            Accumulator::merge(&mut left.state, right.state)?;
        }
        Ok(())
    }

    fn get_persistent(&self) -> Self::Persistent {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_projection_reuses_stats_through_merge_and_restore() {
        let mut state = VectorAccumulatorState::from_config(
            vec!["value".into()],
            TrainingProjection::component("value"),
            None,
            AccumulatorMomentConfig::default(),
        );
        state
            .ingest_vector(&[3.0], &Point::new(vec![], vec![], 2.0))
            .unwrap();
        assert!(std::ptr::eq(state.projection(), &state.components[0]));
        state.merge(state.clone()).unwrap();
        assert_eq!(state.sample_count(), 2);
        let mut saved = serde_json::to_value(&state).unwrap();
        assert_eq!(
            saved["projection"],
            serde_json::json!({"kind":"component", "name":"value"})
        );
        assert_eq!(
            serde_json::from_value::<VectorAccumulatorState>(saved.clone()).unwrap(),
            state
        );
        saved["projection"]["name"] = serde_json::json!("missing");
        assert!(serde_json::from_value::<VectorAccumulatorState>(saved).is_err());
    }
}
