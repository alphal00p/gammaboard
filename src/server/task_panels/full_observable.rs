use crate::core::{
    EngineError, ImageDisplayMode, LineDisplayMode, LineRasterGeometry, PlaneRasterGeometry,
    RunSpec, RunTask,
};
use crate::evaluation::{FullObservableProgress, ObservableState, SemanticObservableKind};
use crate::server::panels::{
    ImageColorMode, PanelHistoryMode, PanelKind, PanelSpec, PanelState, PlotPoint, PlotSeries,
    key_value, key_value_panel, multi_timeseries_panel, panel_spec, progress_panel,
    scalar_timeseries_panel,
};
use serde_json::Value as JsonValue;

pub(super) fn image_panel_specs() -> Vec<PanelSpec> {
    vec![
        progress_spec("image_progress", "Image Progress"),
        completion_spec("image_completion", "Image Completion"),
        panel_spec(
            "image_view",
            "Rendered Image",
            PanelKind::Image2d,
            PanelHistoryMode::None,
        ),
    ]
}

pub(super) fn line_panel_specs(display: LineDisplayMode, run_spec: &RunSpec) -> Vec<PanelSpec> {
    let mut panels = vec![
        progress_spec("line_progress", "Line Progress"),
        completion_spec("line_completion", "Line Completion"),
    ];
    if line_uses_complex_components(display, run_spec) {
        panels.push(panel_spec(
            "line_components",
            "Complex Components",
            PanelKind::MultiTimeseries,
            PanelHistoryMode::None,
        ));
    } else {
        panels.push(panel_spec(
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
            PanelHistoryMode::None,
        ));
    }
    panels
}

pub(super) fn build_image_current_panels(
    task: &RunTask,
    observable: Option<&ObservableState>,
    geometry: &PlaneRasterGeometry,
    display: ImageDisplayMode,
) -> Result<Vec<PanelState>, EngineError> {
    build_current_panels(
        task,
        geometry.nr_points(),
        "image_progress",
        "pixels",
        |total, processed| image_completion_panel(total, processed),
        observable.map(|observable| build_image_view_panel(observable, geometry, display)),
    )
}

pub(super) fn build_line_current_panels(
    task: &RunTask,
    observable: Option<&ObservableState>,
    geometry: &LineRasterGeometry,
    display: LineDisplayMode,
    run_spec: &RunSpec,
) -> Result<Vec<PanelState>, EngineError> {
    build_current_panels(
        task,
        geometry.nr_points(),
        "line_progress",
        "points",
        |total, processed| line_completion_panel(total, processed),
        observable
            .map(|observable| build_line_value_panels(observable, geometry, display, run_spec)),
    )
}

pub(super) fn build_image_panels_from_persisted(
    persisted: &JsonValue,
    geometry: &PlaneRasterGeometry,
) -> Result<Vec<PanelState>, EngineError> {
    build_progress_panels_from_persisted(
        persisted,
        "image_progress",
        geometry.nr_points(),
        "pixels",
        image_completion_panel,
    )
}

pub(super) fn build_line_panels_from_persisted(
    persisted: &JsonValue,
    geometry: &LineRasterGeometry,
) -> Result<Vec<PanelState>, EngineError> {
    build_progress_panels_from_persisted(
        persisted,
        "line_progress",
        geometry.nr_points(),
        "points",
        line_completion_panel,
    )
}

fn progress_spec(panel_id: &str, label: &str) -> PanelSpec {
    panel_spec(panel_id, label, PanelKind::Progress, PanelHistoryMode::None)
}

fn completion_spec(panel_id: &str, label: &str) -> PanelSpec {
    panel_spec(panel_id, label, PanelKind::KeyValue, PanelHistoryMode::None)
}

fn build_current_panels(
    task: &RunTask,
    total: usize,
    progress_panel_id: &str,
    unit: &'static str,
    completion_panel: impl Fn(usize, usize) -> PanelState,
    value_panels: Option<Result<Vec<PanelState>, EngineError>>,
) -> Result<Vec<PanelState>, EngineError> {
    let processed = task.nr_completed_samples.max(0) as usize;
    let mut panels = progress_panels(
        progress_panel_id,
        processed,
        total,
        unit,
        completion_panel(total, processed),
    );
    if let Some(value_panels) = value_panels {
        panels.extend(value_panels?);
    }
    Ok(panels)
}

fn progress_panels(
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

fn build_progress_panels_from_persisted(
    persisted: &JsonValue,
    progress_panel_id: &str,
    total: usize,
    unit: &'static str,
    completion_panel: impl Fn(usize, usize) -> PanelState,
) -> Result<Vec<PanelState>, EngineError> {
    let progress = decode_full_progress(persisted)?;
    Ok(progress_panels(
        progress_panel_id,
        progress.processed,
        total,
        unit,
        completion_panel(total, progress.processed),
    ))
}

fn build_image_view_panel(
    observable: &ObservableState,
    geometry: &PlaneRasterGeometry,
    display: ImageDisplayMode,
) -> Result<Vec<PanelState>, EngineError> {
    let width = geometry.u_linspace.count;
    let height = geometry.v_linspace.count;
    let panel = match observable {
        ObservableState::FullScalar(state) => PanelState::Image2d {
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
        },
        ObservableState::FullComplex(state) => PanelState::Image2d {
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
        },
        other => {
            return Err(EngineError::engine(format!(
                "image task expected full observable, got {}",
                other.kind_str()
            )));
        }
    };
    Ok(vec![panel])
}

fn build_line_value_panels(
    observable: &ObservableState,
    geometry: &LineRasterGeometry,
    display: LineDisplayMode,
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
                .map(point)
                .collect(),
        )]),
        ObservableState::FullComplex(state) => {
            if line_uses_complex_components(display, run_spec) {
                Ok(vec![multi_timeseries_panel(
                    "line_components",
                    vec![
                        PlotSeries {
                            id: "real".to_string(),
                            label: "Real Part".to_string(),
                            points: xs
                                .iter()
                                .copied()
                                .zip(state.values.iter().map(|value| value.re))
                                .map(point)
                                .collect(),
                        },
                        PlotSeries {
                            id: "imag".to_string(),
                            label: "Imaginary Part".to_string(),
                            points: xs
                                .iter()
                                .copied()
                                .zip(state.values.iter().map(|value| value.im))
                                .map(point)
                                .collect(),
                        },
                    ],
                )])
            } else {
                Ok(vec![scalar_timeseries_panel(
                    "line_real",
                    xs.iter()
                        .copied()
                        .zip(state.values.iter().map(|value| value.re))
                        .map(point)
                        .collect(),
                )])
            }
        }
        other => Err(EngineError::engine(format!(
            "line task expected full observable, got {}",
            other.kind_str()
        ))),
    }
}

fn line_uses_complex_components(display: LineDisplayMode, run_spec: &RunSpec) -> bool {
    matches!(
        run_spec.evaluator.observable_kind(),
        SemanticObservableKind::Complex
    ) && matches!(
        display,
        LineDisplayMode::Auto | LineDisplayMode::ComplexComponents
    )
}

fn point((x, y): (f64, f64)) -> PlotPoint {
    PlotPoint {
        x,
        y,
        y_min: None,
        y_max: None,
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

fn decode_full_progress(persisted: &JsonValue) -> Result<FullObservableProgress, EngineError> {
    serde_json::from_value(persisted.clone())
        .map_err(|err| EngineError::build(format!("invalid full observable progress: {err}")))
}

fn line_x_value(geometry: &LineRasterGeometry, index: usize) -> f64 {
    if geometry.linspace.count <= 1 {
        return geometry.linspace.start;
    }
    let t = index as f64 / (geometry.linspace.count - 1) as f64;
    geometry.linspace.start + t * (geometry.linspace.stop - geometry.linspace.start)
}
