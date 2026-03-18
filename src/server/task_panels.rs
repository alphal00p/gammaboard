use crate::core::{
    EngineError, EvaluatorConfig, ImageDisplayMode, LineDisplayMode, RunSpec, RunTask, RunTaskSpec,
};
use crate::evaluation::{FullObservableProgress, ObservableState, SemanticObservableKind};
use crate::server::panels::{
    ImageColorMode, PanelDescriptor, PanelKind, PanelState, PlotPoint, TaskHistoryItem,
    history_item, key_value, key_value_panel, panel_descriptor, progress_panel,
    scalar_timeseries_panel, single_point_band,
};
use crate::stores::{TaskOutputSnapshot, TaskStageSnapshot};
use serde_json::Value as JsonValue;

impl RunTaskSpec {
    pub fn describe_panels(&self, run_spec: &RunSpec) -> Vec<PanelDescriptor> {
        match self {
            Self::Pause => vec![panel_descriptor(
                "pause_state",
                "Pause State",
                PanelKind::Text,
                false,
            )],
            Self::Sample { .. } => describe_sample_panels(run_spec),
            Self::Image { .. } => describe_image_panels(),
            Self::PlotLine { .. } => describe_line_panels(run_spec),
        }
    }

    pub fn build_current_panels(
        &self,
        task: &RunTask,
        observable: Option<&ObservableState>,
        run_spec: &RunSpec,
    ) -> Result<Vec<PanelState>, EngineError> {
        match self {
            Self::Pause => Ok(vec![PanelState::Text {
                panel_id: "pause_state".to_string(),
                text: "Task is paused".to_string(),
            }]),
            Self::Sample { .. } => build_sample_current_panels(task, observable, run_spec),
            Self::Image { geometry, display } => {
                build_image_current_panels(task, observable, geometry, *display)
            }
            Self::PlotLine { geometry, display } => {
                build_line_current_panels(task, observable, geometry, *display, run_spec)
            }
        }
    }

    pub fn build_history_item(
        &self,
        task: &RunTask,
        snapshot: &TaskOutputSnapshot,
        run_spec: &RunSpec,
    ) -> Result<TaskHistoryItem, EngineError> {
        let panels = match self {
            Self::Pause => Vec::new(),
            Self::Sample { nr_samples, .. } => {
                let observable = decode_aggregate_persisted_observable(
                    run_spec.evaluator.observable_kind(),
                    &snapshot.persisted_output,
                )?;
                build_sample_panels(
                    observable.sample_count() as f64,
                    nr_samples.map(|value| value as f64),
                    Some(&observable),
                    run_spec,
                )
            }
            Self::Image { geometry, .. } => persisted_full_progress_panels(
                &snapshot.persisted_output,
                "image_progress",
                geometry.nr_points(),
                "pixels",
                image_completion_panel,
            )?,
            Self::PlotLine { geometry, .. } => persisted_full_progress_panels(
                &snapshot.persisted_output,
                "line_progress",
                geometry.nr_points(),
                "points",
                line_completion_panel,
            )?,
        };

        let _ = task;
        Ok(history_item(
            snapshot.id.clone(),
            snapshot.created_at,
            panels,
        ))
    }

    pub fn build_current_panels_from_persisted(
        &self,
        task: &RunTask,
        persisted: &JsonValue,
        run_spec: &RunSpec,
    ) -> Result<Vec<PanelState>, EngineError> {
        match self {
            Self::Pause => self.build_current_panels(task, None, run_spec),
            Self::Sample { .. } => {
                let observable = decode_aggregate_persisted_observable(
                    run_spec.evaluator.observable_kind(),
                    persisted,
                )?;
                self.build_current_panels(task, Some(&observable), run_spec)
            }
            Self::Image { geometry, .. } => persisted_full_progress_panels(
                persisted,
                "image_progress",
                geometry.nr_points(),
                "pixels",
                image_completion_panel,
            ),
            Self::PlotLine { geometry, .. } => persisted_full_progress_panels(
                persisted,
                "line_progress",
                geometry.nr_points(),
                "points",
                line_completion_panel,
            ),
        }
    }

    pub fn build_current_panels_from_stage_snapshot(
        &self,
        task: &RunTask,
        snapshot: &TaskStageSnapshot,
        run_spec: &RunSpec,
    ) -> Result<Vec<PanelState>, EngineError> {
        self.build_current_panels(task, Some(&snapshot.observable_state), run_spec)
    }
}

fn describe_sample_panels(run_spec: &RunSpec) -> Vec<PanelDescriptor> {
    let mut panels = vec![panel_descriptor(
        "sample_progress",
        "Sample Progress",
        PanelKind::Progress,
        true,
    )];
    panels.push(panel_descriptor(
        "real_estimate_history",
        estimate_label(run_spec),
        PanelKind::ScalarTimeseries,
        true,
    ));
    if matches!(
        run_spec.evaluator.observable_kind(),
        SemanticObservableKind::Complex
    ) {
        panels.push(panel_descriptor(
            "imag_estimate_history",
            "Imaginary Mean",
            PanelKind::ScalarTimeseries,
            true,
        ));
    }
    panels.push(panel_descriptor(
        "abs_signal_to_noise_history",
        "Mean(|x|)^2 / abs_err^2",
        PanelKind::ScalarTimeseries,
        true,
    ));
    panels.push(panel_descriptor(
        "estimate_summary",
        "Estimate Summary",
        PanelKind::KeyValue,
        false,
    ));
    panels
}

fn describe_image_panels() -> Vec<PanelDescriptor> {
    vec![
        panel_descriptor(
            "image_progress",
            "Image Progress",
            PanelKind::Progress,
            true,
        ),
        panel_descriptor(
            "image_completion",
            "Image Completion",
            PanelKind::KeyValue,
            true,
        ),
        panel_descriptor("image_view", "Rendered Image", PanelKind::Image2d, false),
    ]
}

fn full_progress_panels(
    progress_panel_id: &str,
    current: usize,
    total: usize,
    unit: &'static str,
    completion_panel: PanelState,
) -> Vec<PanelState> {
    vec![
        progress_panel(
            progress_panel_id,
            current as f64,
            Some(total as f64),
            Some(unit),
        ),
        completion_panel,
    ]
}

fn persisted_full_progress_panels(
    persisted: &JsonValue,
    progress_panel_id: &str,
    total: usize,
    unit: &'static str,
    completion_panel: impl Fn(usize, usize) -> PanelState,
) -> Result<Vec<PanelState>, EngineError> {
    let progress = decode_full_progress(persisted)?;
    Ok(full_progress_panels(
        progress_panel_id,
        progress.processed,
        total,
        unit,
        completion_panel(total, progress.processed),
    ))
}

fn describe_line_panels(run_spec: &RunSpec) -> Vec<PanelDescriptor> {
    let mut panels = vec![
        panel_descriptor("line_progress", "Line Progress", PanelKind::Progress, true),
        panel_descriptor(
            "line_completion",
            "Line Completion",
            PanelKind::KeyValue,
            true,
        ),
    ];
    panels.push(panel_descriptor(
        "line_real",
        if matches!(
            run_spec.evaluator.observable_kind(),
            SemanticObservableKind::Complex
        ) {
            "Real Part"
        } else {
            "Value"
        },
        PanelKind::ScalarTimeseries,
        false,
    ));
    if matches!(
        run_spec.evaluator.observable_kind(),
        SemanticObservableKind::Complex
    ) {
        panels.push(panel_descriptor(
            "line_imag",
            "Imaginary Part",
            PanelKind::ScalarTimeseries,
            false,
        ));
    }
    panels
}

fn build_sample_current_panels(
    task: &RunTask,
    observable: Option<&ObservableState>,
    run_spec: &RunSpec,
) -> Result<Vec<PanelState>, EngineError> {
    Ok(build_sample_panels(
        task.nr_completed_samples as f64,
        task.task.nr_expected_samples().map(|value| value as f64),
        observable,
        run_spec,
    ))
}

fn build_sample_panels(
    progress_current: f64,
    progress_total: Option<f64>,
    observable: Option<&ObservableState>,
    run_spec: &RunSpec,
) -> Vec<PanelState> {
    let mut panels = vec![progress_panel(
        "sample_progress",
        progress_current,
        progress_total,
        Some("samples"),
    )];
    if let Some(observable) = observable {
        panels.extend(build_estimate_panels_from_observable(observable, run_spec));
        panels.push(build_abs_signal_to_noise_panel_from_observable(observable));
        panels.push(build_summary_panel_from_observable(
            "estimate_summary",
            observable,
        ));
    }
    panels
}

fn build_image_current_panels(
    task: &RunTask,
    observable: Option<&ObservableState>,
    geometry: &crate::core::PlaneRasterGeometry,
    display: ImageDisplayMode,
) -> Result<Vec<PanelState>, EngineError> {
    let total = geometry.nr_points();
    let processed = task.nr_completed_samples.max(0) as usize;
    let mut panels = full_progress_panels(
        "image_progress",
        processed,
        total,
        "pixels",
        image_completion_panel(total, processed),
    );
    if let Some(observable) = observable {
        panels.push(build_image_view_panel(observable, geometry, display)?);
    }
    Ok(panels)
}

fn build_line_current_panels(
    task: &RunTask,
    observable: Option<&ObservableState>,
    geometry: &crate::core::LineRasterGeometry,
    _display: LineDisplayMode,
    run_spec: &RunSpec,
) -> Result<Vec<PanelState>, EngineError> {
    let total = geometry.nr_points();
    let processed = task.nr_completed_samples.max(0) as usize;
    let mut panels = full_progress_panels(
        "line_progress",
        processed,
        total,
        "points",
        line_completion_panel(total, processed),
    );
    if let Some(observable) = observable {
        panels.extend(build_line_value_panels(observable, geometry, run_spec)?);
    }
    Ok(panels)
}

fn build_image_view_panel(
    observable: &ObservableState,
    geometry: &crate::core::PlaneRasterGeometry,
    display: ImageDisplayMode,
) -> Result<PanelState, EngineError> {
    let width = geometry.u_linspace.count;
    let height = geometry.v_linspace.count;
    match observable {
        ObservableState::FullScalar(state) => Ok(PanelState::Image2d {
            panel_id: "image_view".to_string(),
            width,
            height,
            values: state.values.iter().map(|value| *value as f32).collect(),
            imag_values: None,
            x_range: [geometry.u_linspace.start, geometry.u_linspace.stop],
            y_range: [geometry.v_linspace.start, geometry.v_linspace.stop],
            color_mode: match display {
                ImageDisplayMode::ComplexHueIntensity => ImageColorMode::ComplexHueIntensity,
                ImageDisplayMode::Auto | ImageDisplayMode::ScalarHeatmap => {
                    ImageColorMode::ScalarHeatmap
                }
            },
        }),
        ObservableState::FullComplex(state) => Ok(PanelState::Image2d {
            panel_id: "image_view".to_string(),
            width,
            height,
            values: state.values.iter().map(|value| value.re as f32).collect(),
            imag_values: Some(state.values.iter().map(|value| value.im as f32).collect()),
            x_range: [geometry.u_linspace.start, geometry.u_linspace.stop],
            y_range: [geometry.v_linspace.start, geometry.v_linspace.stop],
            color_mode: match display {
                ImageDisplayMode::Auto | ImageDisplayMode::ComplexHueIntensity => {
                    ImageColorMode::ComplexHueIntensity
                }
                ImageDisplayMode::ScalarHeatmap => ImageColorMode::ScalarHeatmap,
            },
        }),
        other => Err(EngineError::engine(format!(
            "image task expected full observable, got {}",
            other.kind_str()
        ))),
    }
}

fn build_line_value_panels(
    observable: &ObservableState,
    geometry: &crate::core::LineRasterGeometry,
    run_spec: &RunSpec,
) -> Result<Vec<PanelState>, EngineError> {
    let xs = (0..geometry.nr_points())
        .map(|idx| line_x_value(geometry, idx))
        .collect::<Vec<_>>();
    match observable {
        ObservableState::FullScalar(state) => Ok(vec![scalar_timeseries_panel(
            "line_real",
            xs.iter()
                .copied()
                .zip(state.values.iter().copied())
                .map(|(x, y)| PlotPoint {
                    x,
                    y,
                    y_min: None,
                    y_max: None,
                })
                .collect(),
        )]),
        ObservableState::FullComplex(state) => {
            let mut panels = vec![scalar_timeseries_panel(
                "line_real",
                xs.iter()
                    .copied()
                    .zip(state.values.iter())
                    .map(|(x, value)| PlotPoint {
                        x,
                        y: value.re,
                        y_min: None,
                        y_max: None,
                    })
                    .collect(),
            )];
            if matches!(
                run_spec.evaluator.observable_kind(),
                SemanticObservableKind::Complex
            ) {
                panels.push(scalar_timeseries_panel(
                    "line_imag",
                    xs.iter()
                        .copied()
                        .zip(state.values.iter())
                        .map(|(x, value)| PlotPoint {
                            x,
                            y: value.im,
                            y_min: None,
                            y_max: None,
                        })
                        .collect(),
                ));
            }
            Ok(panels)
        }
        other => Err(EngineError::engine(format!(
            "line task expected full observable, got {}",
            other.kind_str()
        ))),
    }
}

fn completion_panel(panel_id: &str, total: usize, processed: usize) -> PanelState {
    key_value_panel(
        panel_id,
        vec![
            key_value("processed", "Processed", processed),
            key_value("total", "Total", total),
            key_value(
                "completion",
                "Completion",
                if total > 0 {
                    processed as f64 / total as f64
                } else {
                    0.0
                },
            ),
        ],
    )
}

fn image_completion_panel(total: usize, processed: usize) -> PanelState {
    completion_panel("image_completion", total, processed)
}

fn line_completion_panel(total: usize, processed: usize) -> PanelState {
    completion_panel("line_completion", total, processed)
}

fn decode_aggregate_persisted_observable(
    kind: SemanticObservableKind,
    persisted: &JsonValue,
) -> Result<ObservableState, EngineError> {
    ObservableState::from_aggregate_persistent_json(kind, persisted)
}

fn decode_full_progress(persisted: &JsonValue) -> Result<FullObservableProgress, EngineError> {
    serde_json::from_value(persisted.clone())
        .map_err(|err| EngineError::build(format!("invalid full observable progress: {err}")))
}

fn estimate_label(run_spec: &RunSpec) -> &'static str {
    match run_spec.evaluator.observable_kind() {
        SemanticObservableKind::Scalar => "Mean",
        SemanticObservableKind::Complex => "Real Mean",
    }
}

fn build_estimate_panels_from_observable(
    observable: &ObservableState,
    run_spec: &RunSpec,
) -> Vec<PanelState> {
    match run_spec.evaluator.observable_kind() {
        SemanticObservableKind::Scalar => vec![single_point_band(
            "real_estimate_history",
            observable.sample_count() as f64,
            scalar_estimate(observable),
            Some(scalar_estimate(observable) - scalar_error(observable)),
            Some(scalar_estimate(observable) + scalar_error(observable)),
        )],
        SemanticObservableKind::Complex => complex_estimate_panels(observable),
    }
}

fn build_abs_signal_to_noise_panel_from_observable(observable: &ObservableState) -> PanelState {
    scalar_timeseries_panel(
        "abs_signal_to_noise_history",
        vec![PlotPoint {
            x: observable.sample_count() as f64,
            y: abs_signal_to_noise(observable),
            y_min: None,
            y_max: None,
        }],
    )
}

fn build_summary_panel_from_observable(panel_id: &str, observable: &ObservableState) -> PanelState {
    match observable {
        ObservableState::Scalar(state) => key_value_panel(
            panel_id,
            vec![
                key_value("count", "Count", state.count),
                key_value("mean", "Mean", state.mean()),
                key_value("error", "Error", state.stderr()),
                key_value("mean_abs", "Mean Abs", state.mean_abs()),
                key_value(
                    "signal_to_noise",
                    "Mean(|x|)^2 / abs_err^2",
                    state.signal_to_noise(),
                ),
                key_value("rsd", "RSD", state.rsd()),
            ],
        ),
        ObservableState::Complex(state) => key_value_panel(
            panel_id,
            vec![
                key_value("count", "Count", state.count),
                key_value("real_mean", "Real Mean", state.real_mean()),
                key_value("imag_mean", "Imag Mean", state.imag_mean()),
                key_value("real_error", "Real Error", state.real_stderr()),
                key_value("imag_error", "Imag Error", state.imag_stderr()),
                key_value("abs_mean", "Abs Mean", state.abs_mean()),
                key_value("abs_error", "Abs Error", state.abs_stderr()),
                key_value(
                    "signal_to_noise",
                    "Mean(|x|)^2 / abs_err^2",
                    state.signal_to_noise(),
                ),
                key_value("rsd", "RSD", state.rsd()),
            ],
        ),
        ObservableState::FullScalar(state) => key_value_panel(
            panel_id,
            vec![
                key_value("count", "Count", state.values.len()),
                key_value(
                    "min",
                    "Min",
                    state.values.iter().copied().fold(f64::INFINITY, f64::min),
                ),
                key_value(
                    "max",
                    "Max",
                    state
                        .values
                        .iter()
                        .copied()
                        .fold(f64::NEG_INFINITY, f64::max),
                ),
            ],
        ),
        ObservableState::FullComplex(state) => key_value_panel(
            panel_id,
            vec![
                key_value("count", "Count", state.values.len()),
                key_value(
                    "max_abs",
                    "Max |z|",
                    state
                        .values
                        .iter()
                        .map(|value| (value.re * value.re + value.im * value.im).sqrt())
                        .fold(0.0, f64::max),
                ),
            ],
        ),
    }
}

fn scalar_estimate(observable: &ObservableState) -> f64 {
    match observable {
        ObservableState::Scalar(state) => state.mean(),
        _ => 0.0,
    }
}

fn scalar_error(observable: &ObservableState) -> f64 {
    match observable {
        ObservableState::Scalar(state) => state.stderr(),
        _ => 0.0,
    }
}

fn complex_estimate_panels(observable: &ObservableState) -> Vec<PanelState> {
    match observable {
        ObservableState::Complex(state) => vec![
            single_point_band(
                "real_estimate_history",
                state.count as f64,
                state.real_mean(),
                Some(state.real_mean() - state.real_stderr()),
                Some(state.real_mean() + state.real_stderr()),
            ),
            single_point_band(
                "imag_estimate_history",
                state.count as f64,
                state.imag_mean(),
                Some(state.imag_mean() - state.imag_stderr()),
                Some(state.imag_mean() + state.imag_stderr()),
            ),
        ],
        _ => vec![],
    }
}

fn abs_signal_to_noise(observable: &ObservableState) -> f64 {
    observable.abs_signal_to_noise()
}

fn line_x_value(geometry: &crate::core::LineRasterGeometry, index: usize) -> f64 {
    if geometry.linspace.count <= 1 {
        return geometry.linspace.start;
    }
    let t = index as f64 / (geometry.linspace.count - 1) as f64;
    geometry.linspace.start + t * (geometry.linspace.stop - geometry.linspace.start)
}

impl EvaluatorConfig {
    pub fn observable_kind(&self) -> SemanticObservableKind {
        match self {
            Self::Gammaloop { params } => params.observable_kind,
            Self::SinEvaluator { .. } => SemanticObservableKind::Scalar,
            Self::SincEvaluator { .. } => SemanticObservableKind::Complex,
            Self::Unit { params } => params.observable_kind,
            Self::Symbolica { .. } => SemanticObservableKind::Scalar,
        }
    }
}
