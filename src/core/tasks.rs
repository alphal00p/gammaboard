use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::{BatchTransformConfig, BuildError, ObservableConfig, SamplerAggregatorConfig};
use crate::sampling::{RasterLineSamplerParams, RasterPlaneSamplerParams};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTaskState {
    Pending,
    Active,
    Completed,
    Failed,
}

impl RunTaskState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SampleTaskConfig {
    pub batch_transforms: Option<Vec<BatchTransformConfig>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRefSpec {
    Latest,
    FromName(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SamplerAggregatorSourceSpec {
    Latest(String),
    FromName { from_name: String },
    Config { config: SamplerAggregatorConfig },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ObservableSourceSpec {
    Latest(String),
    FromName { from_name: String },
    Config { config: ObservableConfig },
}

fn validate_source_name(field: &str, from_name: &str) -> Result<(), String> {
    let trimmed = from_name.trim();
    if trimmed.is_empty() {
        return Err(format!("{field}.from_name must be non-empty"));
    }
    if trimmed != from_name {
        return Err(format!(
            "{field}.from_name cannot have leading/trailing whitespace"
        ));
    }
    Ok(())
}

impl SamplerAggregatorSourceSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Latest(value) => {
                if value == "latest" {
                    Ok(())
                } else {
                    Err("sampler_aggregator must be one of: \"latest\", { from_name = ... }, { config = ... }".to_string())
                }
            }
            Self::FromName { from_name } => validate_source_name("sampler_aggregator", from_name),
            Self::Config { .. } => Ok(()),
        }
    }
}

impl ObservableSourceSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Latest(value) => {
                if value == "latest" {
                    Ok(())
                } else {
                    Err("observable must be one of: \"latest\", { from_name = ... }, { config = ... }".to_string())
                }
            }
            Self::FromName { from_name } => validate_source_name("observable", from_name),
            Self::Config { .. } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunTaskSpec {
    Init,
    Sample {
        nr_samples: Option<i64>,
        #[serde(default)]
        sampler_aggregator: Option<SamplerAggregatorSourceSpec>,
        #[serde(default)]
        observable: Option<ObservableSourceSpec>,
        #[serde(default)]
        batch_transforms: Option<Vec<BatchTransformConfig>>,
    },
    Image {
        geometry: PlaneRasterGeometry,
        observable: PlotObservableKind,
        #[serde(default)]
        display: ImageDisplayMode,
    },
    PlotLine {
        geometry: LineRasterGeometry,
        observable: PlotObservableKind,
        #[serde(default)]
        display: LineDisplayMode,
    },
}

impl RunTaskSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Init => Err("init task is reserved and cannot be added manually".to_string()),
            Self::Sample {
                nr_samples: Some(nr_samples),
                ..
            } if *nr_samples < 0 => {
                Err("sample task nr_samples must be a non-negative integer when set".to_string())
            }
            Self::Sample {
                nr_samples: None,
                sampler_aggregator:
                    Some(SamplerAggregatorSourceSpec::Config {
                        config: SamplerAggregatorConfig::HavanaTraining { .. },
                    }),
                ..
            } => Err(
                "sample task with havana_training sampler requires nr_samples for training budget"
                    .to_string(),
            ),
            Self::Sample {
                sampler_aggregator: Some(source),
                ..
            } => source.validate(),
            Self::Sample {
                observable: Some(source),
                ..
            } => source.validate(),
            Self::Image { geometry, .. } => geometry.validate(),
            Self::PlotLine { geometry, .. } => geometry.validate(),
            _ => Ok(()),
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Sample { .. } => "sample",
            Self::Image { .. } => "image",
            Self::PlotLine { .. } => "plot_line",
        }
    }

    pub fn sampler_config(&self) -> Option<SamplerAggregatorConfig> {
        match self {
            Self::Init => None,
            Self::Sample { .. } => None,
            Self::Image { geometry, .. } => Some(SamplerAggregatorConfig::RasterPlane {
                params: RasterPlaneSamplerParams {
                    geometry: geometry.clone(),
                },
            }),
            Self::PlotLine { geometry, .. } => Some(SamplerAggregatorConfig::RasterLine {
                params: RasterLineSamplerParams {
                    geometry: geometry.clone(),
                },
            }),
        }
    }

    pub fn sample_sampler_source(&self) -> Option<SourceRefSpec> {
        match self {
            Self::Init => None,
            Self::Sample {
                sampler_aggregator, ..
            } => match sampler_aggregator {
                None | Some(SamplerAggregatorSourceSpec::Latest(_)) => Some(SourceRefSpec::Latest),
                Some(SamplerAggregatorSourceSpec::FromName { from_name }) => {
                    Some(SourceRefSpec::FromName(from_name.clone()))
                }
                Some(SamplerAggregatorSourceSpec::Config { .. }) => None,
            },
            Self::Image { .. } | Self::PlotLine { .. } => None,
        }
    }

    pub fn sample_sampler_config(&self) -> Option<SamplerAggregatorConfig> {
        match self {
            Self::Init => None,
            Self::Sample {
                sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config { config }),
                ..
            } => Some(config.clone()),
            Self::Sample { .. } => None,
            Self::Image { .. } | Self::PlotLine { .. } => None,
        }
    }

    pub fn batch_transforms_config(&self) -> Option<Vec<BatchTransformConfig>> {
        match self {
            Self::Init => Some(Vec::new()),
            Self::Sample {
                batch_transforms, ..
            } => batch_transforms.clone(),
            Self::Image { .. } | Self::PlotLine { .. } => Some(Vec::new()),
        }
    }

    pub fn sample_observable_source(&self) -> Option<SourceRefSpec> {
        match self {
            Self::Init => None,
            Self::Sample { observable, .. } => match observable {
                None | Some(ObservableSourceSpec::Latest(_)) => Some(SourceRefSpec::Latest),
                Some(ObservableSourceSpec::FromName { from_name }) => {
                    Some(SourceRefSpec::FromName(from_name.clone()))
                }
                Some(ObservableSourceSpec::Config { .. }) => None,
            },
            Self::Image { .. } | Self::PlotLine { .. } => None,
        }
    }

    pub fn source_task_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(SourceRefSpec::FromName(name)) = self.sample_sampler_source() {
            out.push(name);
        }
        if let Some(SourceRefSpec::FromName(name)) = self.sample_observable_source() {
            out.push(name);
        }
        out
    }

    pub fn is_sourceable(&self) -> bool {
        !matches!(self, Self::Init)
    }

    pub fn new_observable_config(&self) -> Result<Option<ObservableConfig>, BuildError> {
        match self {
            Self::Init => Ok(None),
            Self::Sample {
                observable: Some(ObservableSourceSpec::Config { config }),
                ..
            } => Ok(Some(config.clone())),
            Self::Sample { .. } => Ok(None),
            Self::Image { observable, .. } | Self::PlotLine { observable, .. } => {
                Ok(Some(observable.full_config()))
            }
        }
    }

    pub fn nr_expected_samples(&self) -> Option<i64> {
        match self {
            Self::Init => None,
            Self::Sample { nr_samples, .. } => *nr_samples,
            Self::Image { geometry, .. } => Some(geometry.nr_points() as i64),
            Self::PlotLine { geometry, .. } => Some(geometry.nr_points() as i64),
        }
    }
}

pub fn generated_task_name(task: &RunTaskSpec, sequence_nr: i32) -> String {
    format!("{}-{sequence_nr}", task.kind_str())
}

pub trait IntoPreflightTask: Sized {
    fn into_preflight(self) -> Result<Option<Self>, BuildError>;
}

impl IntoPreflightTask for RunTaskSpec {
    fn into_preflight(self) -> Result<Option<Self>, BuildError> {
        self.validate().map_err(BuildError::invalid_input)?;
        match self {
            Self::Init => Ok(None),
            Self::Sample {
                nr_samples,
                sampler_aggregator,
                observable,
                batch_transforms,
            } => Ok(Some(Self::Sample {
                nr_samples: Some(if nr_samples == Some(0) { 0 } else { 1 }),
                sampler_aggregator,
                observable,
                batch_transforms,
            })),
            Self::Image {
                mut geometry,
                observable,
                display,
            } => {
                geometry.reduce_for_preflight(4, 4);
                Ok(Some(Self::Image {
                    geometry,
                    observable,
                    display,
                }))
            }
            Self::PlotLine {
                mut geometry,
                observable,
                display,
            } => {
                geometry.reduce_for_preflight(8);
                Ok(Some(Self::PlotLine {
                    geometry,
                    observable,
                    display,
                }))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageDisplayMode {
    #[default]
    Auto,
    ScalarHeatmap,
    ComplexHueIntensity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineDisplayMode {
    #[default]
    Auto,
    ScalarCurve,
    ComplexComponents,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlotObservableKind {
    Scalar,
    Complex,
}

impl PlotObservableKind {
    pub const fn full_config(self) -> ObservableConfig {
        match self {
            Self::Scalar => ObservableConfig::FullScalar,
            Self::Complex => ObservableConfig::FullComplex,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Linspace {
    pub start: f64,
    pub stop: f64,
    pub count: usize,
}

impl Linspace {
    pub fn validate(&self, label: &str) -> Result<(), String> {
        if self.count == 0 {
            return Err(format!("{label} count must be > 0"));
        }
        if !self.start.is_finite() || !self.stop.is_finite() {
            return Err(format!("{label} bounds must be finite"));
        }
        Ok(())
    }

    pub fn reduce_for_preflight(&mut self, count: usize) {
        self.count = self.count.min(count).max(1);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaneRasterGeometry {
    pub offset: Vec<f64>,
    pub u_vector: Vec<f64>,
    pub v_vector: Vec<f64>,
    pub u_linspace: Linspace,
    pub v_linspace: Linspace,
    #[serde(default)]
    pub discrete: Vec<i64>,
}

impl PlaneRasterGeometry {
    pub fn validate(&self) -> Result<(), String> {
        self.u_linspace.validate("u_linspace")?;
        self.v_linspace.validate("v_linspace")?;
        let dims = self.offset.len();
        if dims == 0 {
            return Err(
                "plane geometry offset must have at least one continuous dimension".to_string(),
            );
        }
        if self.u_vector.len() != dims || self.v_vector.len() != dims {
            return Err("plane geometry vectors must match offset dimensionality".to_string());
        }
        if vector_norm_sq(&self.u_vector) == 0.0 || vector_norm_sq(&self.v_vector) == 0.0 {
            return Err("plane geometry vectors must be non-zero".to_string());
        }
        if !vectors_are_independent(&self.u_vector, &self.v_vector) {
            return Err("plane geometry vectors must not be parallel".to_string());
        }
        Ok(())
    }

    pub fn nr_points(&self) -> usize {
        self.u_linspace.count.saturating_mul(self.v_linspace.count)
    }

    pub fn reduce_for_preflight(&mut self, u_count: usize, v_count: usize) {
        self.u_linspace.reduce_for_preflight(u_count);
        self.v_linspace.reduce_for_preflight(v_count);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LineRasterGeometry {
    pub offset: Vec<f64>,
    pub direction: Vec<f64>,
    pub linspace: Linspace,
    #[serde(default)]
    pub discrete: Vec<i64>,
}

impl LineRasterGeometry {
    pub fn validate(&self) -> Result<(), String> {
        self.linspace.validate("linspace")?;
        let dims = self.offset.len();
        if dims == 0 {
            return Err(
                "line geometry offset must have at least one continuous dimension".to_string(),
            );
        }
        if self.direction.len() != dims {
            return Err("line geometry direction must match offset dimensionality".to_string());
        }
        if vector_norm_sq(&self.direction) == 0.0 {
            return Err("line geometry direction must be non-zero".to_string());
        }
        Ok(())
    }

    pub fn nr_points(&self) -> usize {
        self.linspace.count
    }

    pub fn reduce_for_preflight(&mut self, count: usize) {
        self.linspace.reduce_for_preflight(count);
    }
}

fn vector_norm_sq(values: &[f64]) -> f64 {
    values.iter().map(|value| value * value).sum::<f64>()
}

fn vectors_are_independent(left: &[f64], right: &[f64]) -> bool {
    let left_norm = vector_norm_sq(left).sqrt();
    let right_norm = vector_norm_sq(right).sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        return false;
    }
    let cosine = left
        .iter()
        .zip(right.iter())
        .map(|(l, r)| l * r)
        .sum::<f64>()
        / (left_norm * right_norm);
    (1.0 - cosine.abs()) > 1e-9
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTask {
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_i64_as_string")]
    pub id: i64,
    pub run_id: i32,
    pub name: String,
    pub sequence_nr: i32,
    pub task: RunTaskSpec,
    #[serde(serialize_with = "crate::utils::serde_bigint::serialize_option_i64_as_string")]
    pub spawned_from_snapshot_id: Option<i64>,
    pub state: RunTaskState,
    pub nr_produced_samples: i64,
    pub nr_completed_samples: i64,
    pub failure_reason: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTaskInput {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(flatten)]
    pub task: RunTaskSpec,
}

impl RunTaskInput {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(name) = self.name.as_deref() {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err("task name must be non-empty when set".to_string());
            }
            if trimmed != name {
                return Err("task name cannot have leading or trailing whitespace".to_string());
            }
        }
        self.task.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::{HavanaSamplerParams, NaiveMonteCarloSamplerParams};

    #[test]
    fn sample_task_without_observable_reuses_previous_state() {
        let task = RunTaskSpec::Sample {
            nr_samples: Some(10),
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config {
                config: SamplerAggregatorConfig::NaiveMonteCarlo {
                    params: NaiveMonteCarloSamplerParams::default(),
                },
            }),
            observable: Some(ObservableSourceSpec::Latest("latest".to_string())),
            batch_transforms: Some(Vec::new()),
        };

        assert_eq!(task.new_observable_config().unwrap(), None);
    }

    #[test]
    fn sample_task_rejects_dual_source() {
        let missing = RunTaskSpec::Sample {
            nr_samples: Some(0),
            sampler_aggregator: None,
            observable: None,
            batch_transforms: None,
        };
        assert!(missing.validate().is_ok());

        let both = RunTaskSpec::Sample {
            nr_samples: Some(0),
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Latest("latest".to_string())),
            observable: None,
            batch_transforms: None,
        };
        assert!(both.validate().is_ok());
    }

    #[test]
    fn sample_task_with_havana_training_requires_budget_in_config_mode() {
        let task = RunTaskSpec::Sample {
            nr_samples: None,
            sampler_aggregator: Some(SamplerAggregatorSourceSpec::Config {
                config: SamplerAggregatorConfig::HavanaTraining {
                    params: HavanaSamplerParams::default(),
                },
            }),
            observable: None,
            batch_transforms: None,
        };
        assert!(task.validate().is_err());
    }

    #[test]
    fn plotting_tasks_always_request_fresh_full_observables() {
        let image = RunTaskSpec::Image {
            geometry: PlaneRasterGeometry {
                offset: vec![0.0, 0.0],
                u_vector: vec![1.0, 0.0],
                v_vector: vec![0.0, 1.0],
                u_linspace: Linspace {
                    start: -1.0,
                    stop: 1.0,
                    count: 8,
                },
                v_linspace: Linspace {
                    start: -1.0,
                    stop: 1.0,
                    count: 8,
                },
                discrete: Vec::new(),
            },
            observable: PlotObservableKind::Complex,
            display: ImageDisplayMode::Auto,
        };
        let line = RunTaskSpec::PlotLine {
            geometry: LineRasterGeometry {
                offset: vec![0.0, 0.0],
                direction: vec![1.0, 0.0],
                linspace: Linspace {
                    start: -1.0,
                    stop: 1.0,
                    count: 8,
                },
                discrete: Vec::new(),
            },
            observable: PlotObservableKind::Scalar,
            display: LineDisplayMode::Auto,
        };

        assert_eq!(
            image.new_observable_config().unwrap(),
            Some(ObservableConfig::FullComplex)
        );
        assert_eq!(
            line.new_observable_config().unwrap(),
            Some(ObservableConfig::FullScalar)
        );
    }
}
