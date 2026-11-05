use super::{TaskPanelContext, TaskPanelProjector, panel_projector};
use crate::core::{
    EngineError, ImageDisplayMode, LineDisplayMode, LineRasterGeometry, PlaneRasterGeometry,
    PlotAccumulatorKind,
};
use crate::evaluation::{AccumulatorState, FullAccumulatorProgress};
use crate::sampling::sampler_aggregator::{coprime_stride, permuted_raster_index};
use crate::server::panels::{
    ImageColorMode, ImageNormalizationMode, PanelHistoryMode, PanelKind, PanelSpec, PanelState,
    PanelWidth, PlotPoint, PlotSeries, multi_timeseries_panel, panel_spec, progress_panel,
    scalar_timeseries_panel, select_state_spec, sized_panel_spec, state_option,
};
use serde_json::Value as JsonValue;

pub(super) fn image_projectors(
    geometry: PlaneRasterGeometry,
    display: ImageDisplayMode,
    accumulator: PlotAccumulatorKind,
) -> Vec<TaskPanelProjector> {
    vec![
        progress_projector(
            "image_progress",
            "Image Progress",
            geometry.nr_points(),
            "pixels",
        ),
        image_view_mode_projector(display, accumulator.components()),
        image_view_projector(geometry, display, accumulator.components()),
    ]
}

#[derive(Clone)]
enum ImageViewMode {
    Component(String),
    Magnitude,
    Phase,
}

impl ImageViewMode {
    fn as_str(&self) -> String {
        match self {
            Self::Component(name) => format!("component:{name}"),
            Self::Magnitude => "vector_magnitude".into(),
            Self::Phase => "complex_phase".into(),
        }
    }

    fn panel_spec(display: ImageDisplayMode, components: &[String]) -> PanelSpec {
        let mut spec = panel_spec(
            "image_view_mode",
            "Image component",
            PanelKind::Select,
            PanelHistoryMode::None,
        );
        spec.width = PanelWidth::Compact;
        let mut options = components
            .iter()
            .map(|name| state_option(format!("component:{name}"), name))
            .collect::<Vec<_>>();
        options.push(state_option(
            "vector_magnitude",
            if components.len() == 1 {
                "Absolute value"
            } else {
                "Vector magnitude"
            },
        ));
        if components == ["real", "imag"] {
            options.push(state_option("complex_phase", "Complex phase"));
        }
        spec.state = Some(select_state_spec(
            JsonValue::String(default_image_view_mode(display, components).as_str()),
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
    if line_uses_vector_components(display, &accumulator) {
        projectors.push(line_components_projector(geometry));
    } else {
        let label = if accumulator.components().len() > 1 {
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
        sized_panel_spec(
            panel_id,
            label,
            PanelKind::Progress,
            PanelHistoryMode::None,
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
    components: Vec<String>,
) -> TaskPanelProjector {
    panel_projector(
        sized_panel_spec(
            "image_view",
            "Rendered Image",
            PanelKind::Image2d,
            PanelHistoryMode::None,
            PanelWidth::Full,
        ),
        move |ctx| match ctx.source.accumulator() {
            Some(accumulator) => Ok(Some(image_view_panel(
                accumulator,
                &geometry,
                selected_image_view_mode(ctx, display, &components),
            )?)),
            None => Ok(None),
        },
        |_ctx| Ok(None),
    )
}

fn image_view_mode_projector(
    display: ImageDisplayMode,
    components: Vec<String>,
) -> TaskPanelProjector {
    panel_projector(
        ImageViewMode::panel_spec(display, &components),
        |_ctx| Ok(None),
        |_ctx| Ok(None),
    )
}

fn line_components_projector(geometry: LineRasterGeometry) -> TaskPanelProjector {
    panel_projector(
        sized_panel_spec(
            "line_components",
            "Components",
            PanelKind::MultiTimeseries,
            PanelHistoryMode::None,
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
        sized_panel_spec(
            "line_real",
            label,
            PanelKind::ScalarTimeseries,
            PanelHistoryMode::None,
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
            let (values, imag_values, color_mode, label) = match mode {
                ImageViewMode::Component(name) => (
                    state.component_values(&name).unwrap_or_default(),
                    None,
                    ImageColorMode::ScalarHeatmap,
                    name,
                ),
                ImageViewMode::Magnitude => (
                    state
                        .values_row_major
                        .chunks_exact(state.components.len())
                        .map(|row| row.iter().fold(0.0_f64, |n, x| n.hypot(*x)))
                        .collect(),
                    None,
                    ImageColorMode::ScalarHeatmap,
                    "Vector magnitude".into(),
                ),
                ImageViewMode::Phase => (
                    state.component_values("real").unwrap_or_default(),
                    state.component_values("imag"),
                    ImageColorMode::ComplexPhase,
                    "Complex phase".into(),
                ),
            };
            Ok(PanelState::Image2d {
                panel_id: "image_view".to_string(),
                width,
                height,
                values: reorder_scalar_values(&values, total),
                imag_values: imag_values.map(|values| reorder_scalar_values(&values, total)),
                invalid_indices: None,
                x_range: [geometry.u_linspace.start, geometry.u_linspace.stop],
                y_range: [geometry.v_linspace.start, geometry.v_linspace.stop],
                color_mode,
                normalization_mode: ImageNormalizationMode::MinMax,
                metric_label: Some(label),
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

fn default_image_view_mode(display: ImageDisplayMode, components: &[String]) -> ImageViewMode {
    match display {
        ImageDisplayMode::VectorMagnitude => ImageViewMode::Magnitude,
        ImageDisplayMode::ComplexPhase if components == ["real", "imag"] => ImageViewMode::Phase,
        _ => ImageViewMode::Component(
            components
                .first()
                .cloned()
                .unwrap_or_else(|| "value".into()),
        ),
    }
}

fn selected_image_view_mode(
    ctx: &TaskPanelContext<'_>,
    display: ImageDisplayMode,
    components: &[String],
) -> ImageViewMode {
    match ctx.selected_value("image_view_mode") {
        Some("vector_magnitude") if !components.is_empty() => ImageViewMode::Magnitude,
        Some("complex_phase") if components == ["real", "imag"] => ImageViewMode::Phase,
        Some(value)
            if value
                .strip_prefix("component:")
                .is_some_and(|name| components.iter().any(|c| c == name)) =>
        {
            ImageViewMode::Component(value[10..].into())
        }
        _ => default_image_view_mode(display, components),
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

fn line_uses_vector_components(
    display: LineDisplayMode,
    accumulator: &PlotAccumulatorKind,
) -> bool {
    accumulator.components().len() > 1
        && matches!(display, LineDisplayMode::Auto | LineDisplayMode::Components)
}

fn line_xs(geometry: &LineRasterGeometry) -> Vec<f64> {
    geometry.linspace.values().collect()
}

fn point((x, y): (f64, f64)) -> PlotPoint {
    PlotPoint {
        x,
        y,
        ..Default::default()
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

fn decode_full_progress(persisted: &JsonValue) -> Result<FullAccumulatorProgress, EngineError> {
    serde_json::from_value(persisted.clone())
        .map_err(|err| EngineError::build(format!("invalid full accumulator progress: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::FullVectorAccumulatorState;

    #[test]
    fn named_vector_images_include_every_component_in_the_norm() {
        let components = vec!["x".into(), "y".into(), "z".into()];
        let state = AccumulatorState::FullVector(FullVectorAccumulatorState {
            components: components.clone(),
            values_row_major: vec![3., 4., 12.],
        });
        let geometry = serde_json::from_value(serde_json::json!({
            "offset":[0.,0.],"u_vector":[1.,0.],"v_vector":[0.,1.],
            "u_linspace":{"start":0.,"stop":1.,"count":1},
            "v_linspace":{"start":0.,"stop":1.,"count":1}
        }))
        .unwrap();
        for (mode, expected) in [
            (ImageViewMode::Magnitude, 13.),
            (ImageViewMode::Component("z".into()), 12.),
        ] {
            let PanelState::Image2d {
                values,
                imag_values,
                ..
            } = image_view_panel(&state, &geometry, mode).unwrap()
            else {
                panic!("image")
            };
            assert_eq!(values, vec![expected]);
            assert!(imag_values.is_none());
        }
        let options = serde_json::to_string(&ImageViewMode::panel_spec(
            ImageDisplayMode::Auto,
            &components,
        ))
        .unwrap();
        assert!(options.contains("component:z"));
        assert!(!options.contains("complex_phase"));
        assert!(line_uses_vector_components(
            LineDisplayMode::Auto,
            &PlotAccumulatorKind::Components(components)
        ));
    }
}
