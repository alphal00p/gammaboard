use super::{Accumulator, ScalarAccumulatorState};
use crate::core::{DiscreteProjectionConfig, TrainingProjection};
use crate::evaluation::batch::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamedScalarAccumulator {
    pub name: String,
    pub state: ScalarAccumulatorState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VectorAccumulatorState {
    pub components: Vec<NamedScalarAccumulator>,
    pub projection: NamedScalarAccumulator,
    pub projection_spec: TrainingProjection,
}

impl VectorAccumulatorState {
    pub fn from_config(
        components: Vec<String>,
        projection_spec: TrainingProjection,
        discrete_projections: Option<DiscreteProjectionConfig>,
    ) -> Self {
        let components = components
            .into_iter()
            .map(|name| NamedScalarAccumulator {
                name,
                state: ScalarAccumulatorState::from_config(discrete_projections.clone()),
            })
            .collect();
        Self {
            components,
            projection: NamedScalarAccumulator {
                name: "training_projection".to_string(),
                state: ScalarAccumulatorState::from_config(discrete_projections),
            },
            projection_spec,
        }
    }

    pub fn ingest_vector(&mut self, values: &[f64], point: &Point) -> Result<f64, String> {
        if values.len() != self.components.len() {
            return Err(format!(
                "vector observable value count mismatch: expected {}, got {}",
                self.components.len(),
                values.len()
            ));
        }
        let projected = self.project(values)?;
        for (component, value) in self.components.iter_mut().zip(values.iter()) {
            component.state.add_sample(*value, point);
        }
        self.projection.state.add_sample(projected, point);
        Ok(projected)
    }

    pub fn project(&self, values: &[f64]) -> Result<f64, String> {
        match &self.projection_spec {
            TrainingProjection::Component { name } => {
                let index = self
                    .components
                    .iter()
                    .position(|component| component.name == *name)
                    .ok_or_else(|| {
                        format!("training projection references unknown component {name:?}")
                    })?;
                Ok(values[index])
            }
            TrainingProjection::Norm => {
                Ok(values.iter().map(|value| value * value).sum::<f64>().sqrt())
            }
        }
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
        self.projection.state.rsd()
    }

    pub fn signal_to_noise(&self) -> f64 {
        self.projection.state.signal_to_noise()
    }
}

impl Accumulator for VectorAccumulatorState {
    type Persistent = Self;
    type Digest = Self;

    fn sample_count(&self) -> i64 {
        self.projection.state.sample_count()
    }

    fn merge(&mut self, other: Self) {
        for (left, right) in self.components.iter_mut().zip(other.components) {
            Accumulator::merge(&mut left.state, right.state);
        }
        Accumulator::merge(&mut self.projection.state, other.projection.state);
    }

    fn get_persistent(&self) -> Self::Persistent {
        self.clone()
    }
}
