use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::core::{
    EngineError, ImageDisplayMode, LineDisplayMode, LineRasterGeometry, PlaneRasterGeometry,
    PlotAccumulatorKind,
};
use crate::evaluation::{AccumulatorState, FullAccumulatorProgress};
use crate::server::panels::{
    ImageColorMode, ImageNormalizationMode, PanelHistoryMode, PanelKind, PanelSpec, PanelState,
    PanelWidth, PlotPoint, PlotSeries, multi_timeseries_panel, panel_spec, progress_panel,
    scalar_timeseries_panel, select_state_spec, state_option, with_panel_width,
};
use num::Integer;
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
        image_view_mode_projector(display),
        image_view_projector(geometry, display),
    ]
}

#[derive(Clone, Copy)]
enum ImageViewMode {
    ScalarHeatmapMinMax,
    ScalarHeatmapSymmetric,
    VectorMagnitude,
    ComplexPhase,
}

impl ImageViewMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::ScalarHeatmapMinMax => "scalar_heatmap_min_max",
            Self::ScalarHeatmapSymmetric => "scalar_heatmap_symmetric",
            Self::VectorMagnitude => "vector_magnitude",
            Self::ComplexPhase => "complex_phase",
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
            ImageDisplayMode::Auto
                | ImageDisplayMode::VectorMagnitude
                | ImageDisplayMode::ComplexPhase
        ) {
            options.push(state_option(
                Self::VectorMagnitude.as_str(),
                "Vector Magnitude",
            ));
            options.push(state_option(Self::ComplexPhase.as_str(), "Complex Phase"));
        }
        spec.state = Some(select_state_spec(
            JsonValue::String(default_mode.as_str().to_string()),
            options,
            None,
        ));
        spec
    }
}

pub(super) fn line_projectors(
    geometry: LineRasterGeometry,
    display: LineDisplayMode,
    accumulator: PlotAccumulatorKind,
) -> Vec<TaskPanelProjector> {
    let mut projectors = vec![progress_projector(
        "line_progress",
        "Line Progress",
        geometry.nr_points(),
        "points",
    )];
    if line_uses_vector_components(display, accumulator) {
        projectors.push(line_components_projector(geometry));
    } else {
        let label = if matches!(accumulator, PlotAccumulatorKind::Vector) {
            "Primary Component"
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
                None,
            )))
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
        move |ctx| match ctx.source.accumulator() {
            Some(accumulator) => Ok(Some(image_view_panel(
                accumulator,
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
                "Components",
                PanelKind::MultiTimeseries,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        move |ctx| match ctx.source.accumulator() {
            Some(accumulator) => line_components_panel(accumulator, &geometry),
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
        move |ctx| match ctx.source.accumulator() {
            Some(accumulator) => Ok(line_real_panel(accumulator, &geometry)?),
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
    accumulator: &AccumulatorState,
    geometry: &PlaneRasterGeometry,
    mode: ImageViewMode,
) -> Result<PanelState, EngineError> {
    let width = geometry.u_linspace.count;
    let height = geometry.v_linspace.count;
    let total = geometry.nr_points();
    match accumulator {
        AccumulatorState::FullVector(state) => {
            let real_values = state
                .component_values("real")
                .or_else(|| state.component_values("value"))
                .or_else(|| {
                    state
                        .components
                        .first()
                        .and_then(|name| state.component_values(name))
                })
                .unwrap_or_default();
            let imag_values = state.component_values("imag");
            Ok(PanelState::Image2d {
                panel_id: "image_view".to_string(),
                width,
                height,
                values: reorder_scalar_values(&real_values, total),
                imag_values: imag_values.map(|values| reorder_scalar_values(&values, total)),
                invalid_indices: reordered_invalid_indices(&state.invalid_entries, total),
                x_range: [geometry.u_linspace.start, geometry.u_linspace.stop],
                y_range: [geometry.v_linspace.start, geometry.v_linspace.stop],
                color_mode: image_color_mode(mode),
                normalization_mode: image_normalization_mode(mode),
                metric_label: None,
                metric_mode: None,
                x_label: None,
                y_label: None,
            })
        }
        other => Err(EngineError::engine(format!(
            "image task expected full accumulator, got {}",
            other.kind_str()
        ))),
    }
}

fn default_image_view_mode(display: ImageDisplayMode) -> ImageViewMode {
    match display {
        ImageDisplayMode::VectorMagnitude => ImageViewMode::VectorMagnitude,
        ImageDisplayMode::ComplexPhase => ImageViewMode::ComplexPhase,
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
        Some("vector_magnitude") => ImageViewMode::VectorMagnitude,
        Some("complex_phase") => ImageViewMode::ComplexPhase,
        Some("scalar_heatmap_min_max") => ImageViewMode::ScalarHeatmapMinMax,
        _ => default_image_view_mode(display),
    }
}

fn image_color_mode(mode: ImageViewMode) -> ImageColorMode {
    match mode {
        ImageViewMode::VectorMagnitude => ImageColorMode::VectorMagnitude,
        ImageViewMode::ComplexPhase => ImageColorMode::ComplexPhase,
        ImageViewMode::ScalarHeatmapMinMax | ImageViewMode::ScalarHeatmapSymmetric => {
            ImageColorMode::ScalarHeatmap
        }
    }
}

fn image_normalization_mode(mode: ImageViewMode) -> ImageNormalizationMode {
    match mode {
        ImageViewMode::ScalarHeatmapSymmetric => ImageNormalizationMode::Symmetric,
        ImageViewMode::ScalarHeatmapMinMax
        | ImageViewMode::VectorMagnitude
        | ImageViewMode::ComplexPhase => ImageNormalizationMode::MinMax,
    }
}

fn line_components_panel(
    accumulator: &AccumulatorState,
    geometry: &LineRasterGeometry,
) -> Result<Option<PanelState>, EngineError> {
    let xs = line_xs(geometry);
    match accumulator {
        AccumulatorState::FullVector(state) if state.components.len() > 1 => {
            Ok(Some(multi_timeseries_panel(
                "line_components",
                state
                    .components
                    .iter()
                    .filter_map(|component| {
                        let values = state.component_values(component)?;
                        Some(PlotSeries {
                            id: component.clone(),
                            label: component.clone(),
                            color: None,
                            smooth: None,
                            points: reordered_line_scalar_points(&xs, &values),
                        })
                    })
                    .collect(),
            )))
        }
        AccumulatorState::FullVector(_) => Ok(None),
        other => Err(EngineError::engine(format!(
            "line task expected full accumulator, got {}",
            other.kind_str()
        ))),
    }
}

fn line_real_panel(
    accumulator: &AccumulatorState,
    geometry: &LineRasterGeometry,
) -> Result<Option<PanelState>, EngineError> {
    let xs = line_xs(geometry);
    match accumulator {
        AccumulatorState::FullVector(state) => {
            let values = state
                .component_values("real")
                .or_else(|| state.component_values("value"))
                .or_else(|| {
                    state
                        .components
                        .first()
                        .and_then(|name| state.component_values(name))
                })
                .unwrap_or_default();
            Ok(Some(scalar_timeseries_panel(
                "line_real",
                reordered_line_scalar_points(&xs, &values),
            )))
        }
        other => Err(EngineError::engine(format!(
            "line task expected full accumulator, got {}",
            other.kind_str()
        ))),
    }
}

fn line_uses_vector_components(display: LineDisplayMode, accumulator: PlotAccumulatorKind) -> bool {
    matches!(accumulator, PlotAccumulatorKind::Vector)
        && matches!(display, LineDisplayMode::Auto | LineDisplayMode::Components)
}

fn line_xs(geometry: &LineRasterGeometry) -> Vec<f64> {
    geometry.linspace.values().collect()
}

fn point((x, y): (f64, f64)) -> PlotPoint {
    PlotPoint {
        x,
        y,
        x_sampler_uptime_ms: None,
        x_completed_samples_total: None,
        y_min: None,
        y_max: None,
    }
}

fn reordered_line_scalar_points(xs: &[f64], values: &[f64]) -> Vec<PlotPoint> {
    let total = xs.len();
    let stride = coprime_stride(total);
    values
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(shuffled_index, value)| {
            let canonical_index = permuted_raster_index(shuffled_index, total, stride);
            xs.get(canonical_index).copied().map(|x| point((x, value)))
        })
        .collect()
}

fn reorder_scalar_values(values: &[f64], total: usize) -> Vec<f32> {
    let stride = coprime_stride(total);
    let mut reordered = vec![f32::NAN; total];
    for (shuffled_index, value) in values.iter().copied().enumerate() {
        let canonical_index = permuted_raster_index(shuffled_index, total, stride);
        if let Some(slot) = reordered.get_mut(canonical_index) {
            *slot = value as f32;
        }
    }
    reordered
}

fn reordered_invalid_indices(invalid_entries: &[usize], total: usize) -> Option<Vec<usize>> {
    if invalid_entries.is_empty() {
        return None;
    }
    let stride = coprime_stride(total);
    Some(
        invalid_entries
            .iter()
            .copied()
            .map(|shuffled_index| permuted_raster_index(shuffled_index, total, stride))
            .collect(),
    )
}

fn permuted_raster_index(index: usize, total_samples: usize, stride: usize) -> usize {
    if total_samples <= 1 {
        return index.min(total_samples.saturating_sub(1));
    }
    (index * stride) % total_samples
}

fn coprime_stride(total_samples: usize) -> usize {
    if total_samples <= 1 {
        return 1;
    }

    let phi_minus_one = 0.618_033_988_749_894_9_f64;
    let mut candidate =
        ((total_samples as f64 * phi_minus_one).floor() as usize).clamp(1, total_samples - 1);
    while candidate.gcd(&total_samples) != 1 {
        candidate += 1;
        if candidate >= total_samples {
            candidate = 1;
        }
    }
    candidate
}

fn decode_full_progress(persisted: &JsonValue) -> Result<FullAccumulatorProgress, EngineError> {
    serde_json::from_value(persisted.clone())
        .map_err(|err| EngineError::build(format!("invalid full accumulator progress: {err}")))
}
