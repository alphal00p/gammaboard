use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::core::{
    EngineError, ImageDisplayMode, LineDisplayMode, LineRasterGeometry, PlaneRasterGeometry,
    RunSpec,
};
use crate::evaluation::{FullObservableProgress, ObservableState, SemanticObservableKind};
use crate::server::panels::{
    ImageColorMode, ImageNormalizationMode, PanelHistoryMode, PanelKind, PanelSpec, PanelState,
    PanelWidth, PlotPoint, PlotSeries, key_value, key_value_panel, multi_timeseries_panel,
    panel_spec, progress_panel, scalar_timeseries_panel, select_state_spec, state_option,
    with_panel_width,
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
        image_view_mode_projector(display),
        image_view_projector(geometry, display),
    ]
}

#[derive(Clone, Copy)]
enum ImageViewMode {
    ScalarHeatmapMinMax,
    ScalarHeatmapSymmetric,
    ComplexHueIntensity,
}

impl ImageViewMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::ScalarHeatmapMinMax => "scalar_heatmap_min_max",
            Self::ScalarHeatmapSymmetric => "scalar_heatmap_symmetric",
            Self::ComplexHueIntensity => "complex_hue_intensity",
        }
    }

    fn panel_spec(default_mode: Self, display: ImageDisplayMode) -> PanelSpec {
        let mut spec = panel_spec(
            "image_view_mode",
            "Image View Mode",
            PanelKind::Select,
            PanelHistoryMode::None,
        );
        spec.width = PanelWidth::Compact;
        let mut options = vec![
            state_option(Self::ScalarHeatmapMinMax.as_str(), "Heatmap / Min-Max"),
            state_option(Self::ScalarHeatmapSymmetric.as_str(), "Heatmap / Symmetric"),
        ];
        if matches!(
            display,
            ImageDisplayMode::Auto | ImageDisplayMode::ComplexHueIntensity
        ) {
            options.push(state_option(
                Self::ComplexHueIntensity.as_str(),
                "Complex Hue / Intensity",
            ));
        }
        spec.state = Some(select_state_spec(
            JsonValue::String(default_mode.as_str().to_string()),
            options,
        ));
        spec
    }
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
        with_panel_width(
            panel_spec(panel_id, label, PanelKind::Progress, PanelHistoryMode::None),
            PanelWidth::Full,
        ),
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
        with_panel_width(
            panel_spec(panel_id, label, PanelKind::KeyValue, PanelHistoryMode::None),
            PanelWidth::Compact,
        ),
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
        with_panel_width(
            panel_spec(
                "image_view",
                "Rendered Image",
                PanelKind::Image2d,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        move |ctx| match ctx.source.observable() {
            Some(observable) => Ok(Some(image_view_panel(
                observable,
                &geometry,
                selected_image_view_mode(ctx, display),
            )?)),
            None => Ok(None),
        },
        |_ctx| Ok(None),
    )
}

fn image_view_mode_projector(display: ImageDisplayMode) -> TaskPanelProjector {
    let default_mode = default_image_view_mode(display);
    panel_projector(
        ImageViewMode::panel_spec(default_mode, display),
        |_ctx| Ok(None),
        |_ctx| Ok(None),
    )
}

fn line_components_projector(geometry: LineRasterGeometry) -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                "line_components",
                "Complex Components",
                PanelKind::MultiTimeseries,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
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
        with_panel_width(
            panel_spec(
                "line_real",
                label,
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
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
    mode: ImageViewMode,
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
            color_mode: image_color_mode(mode),
            normalization_mode: image_normalization_mode(mode),
        }),
        ObservableState::FullComplex(state) => Ok(PanelState::Image2d {
            panel_id: "image_view".to_string(),
            width,
            height,
            values: state.values.iter().map(|value| value.re as f32).collect(),
            imag_values: Some(state.values.iter().map(|value| value.im as f32).collect()),
            x_range: [geometry.u_linspace.start, geometry.u_linspace.stop],
            y_range: [geometry.v_linspace.start, geometry.v_linspace.stop],
            color_mode: image_color_mode(mode),
            normalization_mode: image_normalization_mode(mode),
        }),
        other => Err(EngineError::engine(format!(
            "image task expected full observable, got {}",
            other.kind_str()
        ))),
    }
}

fn default_image_view_mode(display: ImageDisplayMode) -> ImageViewMode {
    match display {
        ImageDisplayMode::ComplexHueIntensity => ImageViewMode::ComplexHueIntensity,
        ImageDisplayMode::Auto | ImageDisplayMode::ScalarHeatmap => {
            ImageViewMode::ScalarHeatmapMinMax
        }
    }
}

fn selected_image_view_mode(
    ctx: &TaskPanelContext<'_>,
    display: ImageDisplayMode,
) -> ImageViewMode {
    match ctx.selected_value("image_view_mode") {
        Some("scalar_heatmap_symmetric") => ImageViewMode::ScalarHeatmapSymmetric,
        Some("complex_hue_intensity") => ImageViewMode::ComplexHueIntensity,
        Some("scalar_heatmap_min_max") => ImageViewMode::ScalarHeatmapMinMax,
        _ => default_image_view_mode(display),
    }
}

fn image_color_mode(mode: ImageViewMode) -> ImageColorMode {
    match mode {
        ImageViewMode::ComplexHueIntensity => ImageColorMode::ComplexHueIntensity,
        ImageViewMode::ScalarHeatmapMinMax | ImageViewMode::ScalarHeatmapSymmetric => {
            ImageColorMode::ScalarHeatmap
        }
    }
}

fn image_normalization_mode(mode: ImageViewMode) -> ImageNormalizationMode {
    match mode {
        ImageViewMode::ScalarHeatmapSymmetric => ImageNormalizationMode::Symmetric,
        ImageViewMode::ScalarHeatmapMinMax | ImageViewMode::ComplexHueIntensity => {
            ImageNormalizationMode::MinMax
        }
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
