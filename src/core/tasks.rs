use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::{
    BuildError, ObservableConfig, ParametrizationConfig, RunSpec, SamplerAggregatorConfig,
};
use crate::evaluation::ObservableState;
use crate::sampling::{
    IdentityParametrizationParams, RasterLineSamplerParams, RasterPlaneSamplerParams,
};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshotRef {
    pub run_id: i32,
    pub task_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunTaskInputSpec {
    Sample {
        nr_samples: Option<i64>,
        sampler_aggregator: Option<SamplerAggregatorConfig>,
        parametrization: Option<ParametrizationConfig>,
        #[serde(default)]
        observable: Option<ObservableConfig>,
        #[serde(default)]
        start_from: Option<TaskSnapshotRef>,
    },
    Image {
        geometry: PlaneRasterGeometry,
        #[serde(default)]
        display: ImageDisplayMode,
        #[serde(default)]
        start_from: Option<TaskSnapshotRef>,
    },
    PlotLine {
        geometry: LineRasterGeometry,
        #[serde(default)]
        display: LineDisplayMode,
        #[serde(default)]
        start_from: Option<TaskSnapshotRef>,
    },
    Pause,
}

impl RunTaskInputSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Sample {
                nr_samples: Some(nr_samples),
                ..
            } if *nr_samples <= 0 => {
                Err("sample task nr_samples must be a positive integer when set".to_string())
            }
            Self::Image { geometry, .. } => geometry.validate(),
            Self::PlotLine { geometry, .. } => geometry.validate(),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunTaskSpec {
    Sample {
        nr_samples: Option<i64>,
        sampler_aggregator: SamplerAggregatorConfig,
        parametrization: ParametrizationConfig,
        observable: Option<ObservableConfig>,
        start_from: Option<TaskSnapshotRef>,
    },
    Image {
        geometry: PlaneRasterGeometry,
        #[serde(default)]
        display: ImageDisplayMode,
        #[serde(default)]
        start_from: Option<TaskSnapshotRef>,
    },
    PlotLine {
        geometry: LineRasterGeometry,
        #[serde(default)]
        display: LineDisplayMode,
        #[serde(default)]
        start_from: Option<TaskSnapshotRef>,
    },
    Pause,
}

impl RunTaskSpec {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Sample {
                nr_samples: Some(nr_samples),
                ..
            } if *nr_samples <= 0 => {
                Err("sample task nr_samples must be a positive integer when set".to_string())
            }
            Self::Sample {
                nr_samples: None,
                sampler_aggregator: SamplerAggregatorConfig::HavanaTraining { .. },
                ..
            } => Err(
                "sample task with havana_training sampler requires nr_samples for training budget"
                    .to_string(),
            ),
            Self::Image { geometry, .. } => geometry.validate(),
            Self::PlotLine { geometry, .. } => geometry.validate(),
            _ => Ok(()),
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Sample { .. } => "sample",
            Self::Image { .. } => "image",
            Self::PlotLine { .. } => "plot_line",
            Self::Pause => "pause",
        }
    }

    pub fn sampler_config(&self) -> Option<SamplerAggregatorConfig> {
        match self {
            Self::Sample {
                sampler_aggregator, ..
            } => Some(sampler_aggregator.clone()),
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
            Self::Pause => None,
        }
    }

    pub fn start_from(&self) -> Option<&TaskSnapshotRef> {
        match self {
            Self::Sample { start_from, .. }
            | Self::Image { start_from, .. }
            | Self::PlotLine { start_from, .. } => start_from.as_ref(),
            Self::Pause => None,
        }
    }

    pub fn parametrization_config(&self) -> Option<ParametrizationConfig> {
        match self {
            Self::Sample {
                parametrization, ..
            } => Some(parametrization.clone()),
            Self::Image { .. } | Self::PlotLine { .. } => Some(ParametrizationConfig::Identity {
                params: IdentityParametrizationParams::default(),
            }),
            Self::Pause => None,
        }
    }

    pub fn explicit_observable_config(
        &self,
        base_observable: &ObservableConfig,
    ) -> Option<ObservableConfig> {
        match self {
            Self::Sample { observable, .. } => observable.clone(),
            Self::Image { .. } | Self::PlotLine { .. } => match base_observable {
                ObservableConfig::Scalar | ObservableConfig::FullScalar => {
                    Some(ObservableConfig::FullScalar)
                }
                ObservableConfig::Complex | ObservableConfig::FullComplex => {
                    Some(ObservableConfig::FullComplex)
                }
            },
            Self::Pause => None,
        }
    }

    pub fn observable_config(&self, current_observable: &ObservableConfig) -> ObservableConfig {
        match self {
            Self::Sample {
                observable: Some(observable),
                ..
            } => observable.clone(),
            Self::Sample {
                observable: None, ..
            } => match current_observable {
                ObservableConfig::FullScalar => ObservableConfig::Scalar,
                ObservableConfig::FullComplex => ObservableConfig::Complex,
                other => other.clone(),
            },
            Self::Image { .. } | Self::PlotLine { .. } => self
                .explicit_observable_config(current_observable)
                .expect("image and plot_line tasks always resolve to a full observable"),
            Self::Pause => current_observable.clone(),
        }
    }

    pub fn empty_observable_state(
        &self,
        run_spec: &RunSpec,
        current_observable: &ObservableConfig,
    ) -> Result<ObservableState, BuildError> {
        run_spec
            .evaluator
            .empty_observable_state(&self.observable_config(current_observable))
    }

    pub fn nr_expected_samples(&self) -> Option<i64> {
        match self {
            Self::Sample { nr_samples, .. } => *nr_samples,
            Self::Image { geometry, .. } => Some(geometry.nr_points() as i64),
            Self::PlotLine { geometry, .. } => Some(geometry.nr_points() as i64),
            Self::Pause => None,
        }
    }
}

pub trait IntoPreflightTask: Sized {
    fn into_preflight(self) -> Result<Option<Self>, BuildError>;
}

impl IntoPreflightTask for RunTaskSpec {
    fn into_preflight(self) -> Result<Option<Self>, BuildError> {
        self.validate().map_err(BuildError::invalid_input)?;
        match self {
            Self::Pause => Ok(None),
            Self::Sample {
                sampler_aggregator,
                parametrization,
                observable,
                start_from,
                ..
            } => Ok(Some(Self::Sample {
                nr_samples: Some(1),
                sampler_aggregator,
                parametrization,
                observable,
                start_from,
            })),
            Self::Image {
                mut geometry,
                display,
                start_from,
            } => {
                geometry.reduce_for_preflight(4, 4);
                Ok(Some(Self::Image {
                    geometry,
                    display,
                    start_from,
                }))
            }
            Self::PlotLine {
                mut geometry,
                display,
                start_from,
            } => {
                geometry.reduce_for_preflight(8);
                Ok(Some(Self::PlotLine {
                    geometry,
                    display,
                    start_from,
                }))
            }
        }
    }
}

pub fn resolve_task_queue(
    base_sampler_aggregator: &SamplerAggregatorConfig,
    base_parametrization: &ParametrizationConfig,
    tasks: &[RunTaskInputSpec],
) -> Result<Vec<RunTaskSpec>, String> {
    let mut resolved = Vec::with_capacity(tasks.len());
    let mut current_sampler_aggregator = base_sampler_aggregator.clone();
    let mut current_parametrization = base_parametrization.clone();

    for task in tasks {
        task.validate()?;
        match task {
            RunTaskInputSpec::Pause => resolved.push(RunTaskSpec::Pause),
            RunTaskInputSpec::Sample {
                nr_samples,
                sampler_aggregator,
                parametrization,
                observable,
                start_from,
            } => {
                if let Some(sampler_aggregator) = sampler_aggregator.as_ref() {
                    current_sampler_aggregator = sampler_aggregator.clone();
                }
                if let Some(parametrization) = parametrization.as_ref() {
                    current_parametrization = parametrization.clone();
                }
                resolved.push(RunTaskSpec::Sample {
                    nr_samples: *nr_samples,
                    sampler_aggregator: current_sampler_aggregator.clone(),
                    parametrization: current_parametrization.clone(),
                    observable: observable.clone(),
                    start_from: start_from.clone(),
                });
            }
            RunTaskInputSpec::Image {
                geometry,
                display,
                start_from,
            } => {
                resolved.push(RunTaskSpec::Image {
                    geometry: geometry.clone(),
                    display: *display,
                    start_from: start_from.clone(),
                });
            }
            RunTaskInputSpec::PlotLine {
                geometry,
                display,
                start_from,
            } => {
                resolved.push(RunTaskSpec::PlotLine {
                    geometry: geometry.clone(),
                    display: *display,
                    start_from: start_from.clone(),
                });
            }
        }
    }

    Ok(resolved)
}

pub fn resolve_initial_sampler_aggregator(
    tasks: Option<&[RunTaskInputSpec]>,
    fallback: Option<&SamplerAggregatorConfig>,
) -> Option<SamplerAggregatorConfig> {
    tasks
        .and_then(|tasks| {
            tasks.iter().find_map(|task| match task {
                RunTaskInputSpec::Sample {
                    sampler_aggregator, ..
                } => sampler_aggregator.clone(),
                RunTaskInputSpec::Image { geometry, .. } => {
                    Some(SamplerAggregatorConfig::RasterPlane {
                        params: RasterPlaneSamplerParams {
                            geometry: geometry.clone(),
                        },
                    })
                }
                RunTaskInputSpec::PlotLine { geometry, .. } => {
                    Some(SamplerAggregatorConfig::RasterLine {
                        params: RasterLineSamplerParams {
                            geometry: geometry.clone(),
                        },
                    })
                }
                RunTaskInputSpec::Pause => None,
            })
        })
        .or_else(|| fallback.cloned())
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
    pub id: i64,
    pub run_id: i32,
    pub sequence_nr: i32,
    pub task: RunTaskSpec,
    pub spawned_from_run_id: Option<i32>,
    pub spawned_from_task_id: Option<i64>,
    pub state: RunTaskState,
    pub nr_produced_samples: i64,
    pub nr_completed_samples: i64,
    pub failure_reason: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
