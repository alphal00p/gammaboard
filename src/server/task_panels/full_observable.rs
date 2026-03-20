use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::core::{
    EngineError, ImageDisplayMode, LineDisplayMode, LineRasterGeometry, PlaneRasterGeometry,
    RunSpec,
};
use crate::evaluation::{FullObservableProgress, ObservableState, SemanticObservableKind};
use crate::server::panels::{
    ImageColorMode, PanelHistoryMode, PanelKind, PanelState, PlotPoint, PlotSeries, key_value,
    key_value_panel, multi_timeseries_panel, panel_spec, progress_panel, scalar_timeseries_panel,
};
use serde_json::Value as JsonValue;

pub(super) fn image_projectors(
    geometry: PlaneRasterGeometry,
    display: ImageDisplayMode,
) -> Vec<TaskPanelProjector> {
    vec![
        progress_projector(
            "image_progress",
            "Image Progress",
            geometry.nr_points(),
            "pixels",
        ),
        completion_projector("image_completion", "Image Completion", geometry.nr_points()),
        image_view_projector(geometry, display),
    ]
}

pub(super) fn line_projectors(
    geometry: LineRasterGeometry,
    display: LineDisplayMode,
    run_spec: &RunSpec,
) -> Vec<TaskPanelProjector> {
    let mut projectors = vec![
        progress_projector(
            "line_progress",
            "Line Progress",
            geometry.nr_points(),
            "points",
        ),
        completion_projector("line_completion", "Line Completion", geometry.nr_points()),
    ];
    if line_uses_complex_components(display, run_spec) {
        projectors.push(line_components_projector(geometry));
    } else {
        let label = if matches!(
            run_spec.evaluator.observable_kind(),
            SemanticObservableKind::Complex
        ) {
            "Real Part"
        } else {
            "Value"
        };
        projectors.push(line_real_projector(geometry, label));
    }
    projectors
}

fn progress_projector(
    panel_id: &'static str,
    label: &'static str,
    total: usize,
    unit: &'static str,
) -> TaskPanelProjector {
    panel_projector(
        panel_spec(panel_id, label, PanelKind::Progress, PanelHistoryMode::None),
        move |ctx| {
            let processed = current_processed(ctx, total)?;
            Ok(Some(progress_panel(
                panel_id,
                processed as f64,
                Some(total as f64),
                Some(unit),
            )))
        },
        |_ctx| Ok(None),
    )
}

fn completion_projector(
    panel_id: &'static str,
    label: &'static str,
    total: usize,
) -> TaskPanelProjector {
    panel_projector(
        panel_spec(panel_id, label, PanelKind::KeyValue, PanelHistoryMode::None),
        move |ctx| {
            let processed = current_processed(ctx, total)?;
            Ok(Some(completion_panel(panel_id, total, processed)))
        },
        |_ctx| Ok(None),
    )
}

fn image_view_projector(
    geometry: PlaneRasterGeometry,
    display: ImageDisplayMode,
) -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            "image_view",
            "Rendered Image",
            PanelKind::Image2d,
            PanelHistoryMode::None,
        ),
        move |ctx| match ctx.source.observable() {
            Some(observable) => Ok(Some(image_view_panel(observable, &geometry, display)?)),
            None => Ok(None),
        },
        |_ctx| Ok(None),
    )
}

fn line_components_projector(geometry: LineRasterGeometry) -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            "line_components",
            "Complex Components",
            PanelKind::MultiTimeseries,
            PanelHistoryMode::None,
        ),
        move |ctx| match ctx.source.observable() {
            Some(observable) => line_components_panel(observable, &geometry),
            None => Ok(None),
        },
        |_ctx| Ok(None),
    )
}

fn line_real_projector(geometry: LineRasterGeometry, label: &'static str) -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            "line_real",
            label,
            PanelKind::ScalarTimeseries,
            PanelHistoryMode::None,
        ),
        move |ctx| match ctx.source.observable() {
            Some(observable) => Ok(line_real_panel(observable, &geometry)?),
            None => Ok(None),
        },
        |_ctx| Ok(None),
    )
}

fn current_processed(ctx: &TaskPanelContext<'_>, total: usize) -> Result<usize, EngineError> {
    match ctx.source.persisted() {
        Some(persisted) => Ok(decode_full_progress(persisted)?.processed),
        None => Ok((ctx.task.nr_completed_samples.max(0) as usize).min(total)),
    }
}

fn image_view_panel(
    observable: &ObservableState,
    geometry: &PlaneRasterGeometry,
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

fn line_components_panel(
    observable: &ObservableState,
    geometry: &LineRasterGeometry,
) -> Result<Option<PanelState>, EngineError> {
    let xs = line_xs(geometry);
    match observable {
        ObservableState::FullComplex(state) => Ok(Some(multi_timeseries_panel(
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
        ))),
        ObservableState::FullScalar(_) => Ok(None),
        other => Err(EngineError::engine(format!(
            "line task expected full observable, got {}",
            other.kind_str()
        ))),
    }
}

fn line_real_panel(
    observable: &ObservableState,
    geometry: &LineRasterGeometry,
) -> Result<Option<PanelState>, EngineError> {
    let xs = line_xs(geometry);
    match observable {
        ObservableState::FullScalar(state) => Ok(Some(scalar_timeseries_panel(
            "line_real",
            xs.iter()
                .copied()
                .zip(state.values.iter().copied())
                .map(point)
                .collect(),
        ))),
        ObservableState::FullComplex(state) => Ok(Some(scalar_timeseries_panel(
            "line_real",
            xs.iter()
                .copied()
                .zip(state.values.iter().map(|value| value.re))
                .map(point)
                .collect(),
        ))),
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

fn line_xs(geometry: &LineRasterGeometry) -> Vec<f64> {
    (0..geometry.nr_points())
        .map(|idx| line_x_value(geometry, idx))
        .collect()
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
