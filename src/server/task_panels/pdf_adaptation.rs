use super::{
    TaskPanelContext, TaskPanelCurrentSourcePolicy, TaskPanelProjector, panel_projector_with_source,
};
use crate::core::{
    EngineError, ImageDisplayMode, LineDisplayMode, LineRasterGeometry, PlaneRasterGeometry,
};
use crate::sampling::PdfAdaptationImagePersistedOutput;
use crate::server::panels::{
    HistogramBin, ImageColorMode, ImageNormalizationMode, PanelHistoryMode, PanelKind, PanelState,
    PanelWidth, PlotPoint, key_value, key_value_panel, panel_spec, progress_panel,
    scalar_timeseries_panel, select_state_spec, state_option, with_panel_width,
};
use serde_json::{Value as JsonValue, json};

const HISTOGRAM_BIN_COUNT: usize = 31;

pub(super) fn projectors(
    geometry: PlaneRasterGeometry,
    _display: ImageDisplayMode,
) -> Vec<TaskPanelProjector> {
    vec![
        progress_projector(geometry.nr_points(), "Image Progress", "pixels"),
        image_projector(
            "pdf_adaptation_log_integrand",
            "Reference-normalized integrand",
            PanelWidth::Half,
            geometry.clone(),
            ImageKind::LogPlaneNormalizedIntegrand,
        ),
        image_projector(
            "pdf_adaptation_log_pdf",
            "Normalized PDF",
            PanelWidth::Half,
            geometry.clone(),
            ImageKind::LogPlaneNormalizedPdf,
        ),
        histogram_projector(
            "pdf_adaptation_log_integrand_histogram",
            "Histogram: Reference-normalized integrand",
            PanelWidth::Half,
            ImageKind::LogPlaneNormalizedIntegrand,
        ),
        histogram_projector(
            "pdf_adaptation_log_pdf_histogram",
            "Histogram: Normalized PDF",
            PanelWidth::Half,
            ImageKind::LogPlaneNormalizedPdf,
        ),
        oversampling_metric_projector(),
        image_projector(
            "pdf_adaptation_oversampling",
            "Sampling Accuracy (Plane-Normalized)",
            PanelWidth::Half,
            geometry.clone(),
            ImageKind::OversamplingLegacy,
        ),
        image_projector(
            "pdf_adaptation_oversampling_plane_normalized",
            "Sampling Accuracy (Global-Normalized)",
            PanelWidth::Half,
            geometry,
            ImageKind::OversamplingPlaneNormalized,
        ),
        histogram_projector(
            "pdf_adaptation_oversampling_histogram",
            "Histogram: Sampling Accuracy (Plane-Normalized)",
            PanelWidth::Half,
            ImageKind::OversamplingLegacy,
        ),
        histogram_projector(
            "pdf_adaptation_oversampling_plane_normalized_histogram",
            "Histogram: Sampling Accuracy (Global-Normalized)",
            PanelWidth::Half,
            ImageKind::OversamplingPlaneNormalized,
        ),
        plane_oversampling_scalar_projector(),
    ]
}

fn plane_oversampling_scalar_projector() -> TaskPanelProjector {
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "pdf_adaptation_plane_oversampling_scalar",
                "Plane Oversampling (Global Norm)",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let Some(derived) = current_derived(ctx)? else {
                return Ok(None);
            };
            let mut entries = vec![key_value(
                "global_pdf_norm",
                "Global PDF Norm (Z)",
                derived.output.global_pdf_norm,
            )];
            if let Some(global_abs_integrand_norm) = derived.output.global_abs_integrand_norm {
                entries.push(key_value(
                    "global_abs_integrand_norm",
                    "Global Integrand Norm (I)",
                    global_abs_integrand_norm,
                ));
            }
            if let Some(reference_abs_integrand_norm) = derived.reference_abs_integrand_norm {
                entries.push(key_value(
                    "reference_abs_integrand_norm",
                    "Reference Integrand Norm",
                    reference_abs_integrand_norm,
                ));
            }
            if let Some((factor, count)) = derived.global_plane_oversampling_factor() {
                entries.push(key_value("factor", "Oversampling Factor", factor));
                entries.push(key_value("log10_factor", "log10(Factor)", factor.log10()));
                entries.push(key_value("samples", "Finite Samples", count));
            }
            Ok(Some(key_value_panel(
                "pdf_adaptation_plane_oversampling_scalar",
                entries,
            )))
        },
        |_ctx| Ok(None),
    )
}

pub(super) fn line_projectors(
    geometry: LineRasterGeometry,
    _display: LineDisplayMode,
) -> Vec<TaskPanelProjector> {
    vec![
        progress_projector(geometry.nr_points(), "Line Progress", "points"),
        line_projector(
            "pdf_adaptation_log_integrand_line",
            "Reference-normalized integrand (1D)",
            PanelWidth::Half,
            geometry.clone(),
            ImageKind::LogPlaneNormalizedIntegrand,
        ),
        line_projector(
            "pdf_adaptation_log_pdf_line",
            "Normalized PDF (1D)",
            PanelWidth::Half,
            geometry.clone(),
            ImageKind::LogPlaneNormalizedPdf,
        ),
        oversampling_metric_projector(),
        line_projector(
            "pdf_adaptation_oversampling_line",
            "Sampling Accuracy (Plane-Normalized, 1D)",
            PanelWidth::Half,
            geometry.clone(),
            ImageKind::OversamplingLegacy,
        ),
        line_projector(
            "pdf_adaptation_oversampling_plane_normalized_line",
            "Sampling Accuracy (Global-Normalized, 1D)",
            PanelWidth::Half,
            geometry,
            ImageKind::OversamplingPlaneNormalized,
        ),
        histogram_projector(
            "pdf_adaptation_log_integrand_histogram",
            "Histogram: Reference-normalized integrand",
            PanelWidth::Half,
            ImageKind::LogPlaneNormalizedIntegrand,
        ),
        histogram_projector(
            "pdf_adaptation_log_pdf_histogram",
            "Histogram: Normalized PDF",
            PanelWidth::Half,
            ImageKind::LogPlaneNormalizedPdf,
        ),
        histogram_projector(
            "pdf_adaptation_oversampling_histogram",
            "Histogram: Sampling Accuracy (Plane-Normalized)",
            PanelWidth::Half,
            ImageKind::OversamplingLegacy,
        ),
        histogram_projector(
            "pdf_adaptation_oversampling_plane_normalized_histogram",
            "Histogram: Sampling Accuracy (Global-Normalized)",
            PanelWidth::Half,
            ImageKind::OversamplingPlaneNormalized,
        ),
    ]
}

#[derive(Clone, Copy)]
enum ImageKind {
    LogPlaneNormalizedIntegrand,
    LogPlaneNormalizedPdf,
    OversamplingLegacy,
    OversamplingPlaneNormalized,
}

#[derive(Clone, Copy)]
enum OversamplingMetric {
    RelativeMismatch,
    Log10Ratio,
}

impl OversamplingMetric {
    fn as_str(self) -> &'static str {
        match self {
            Self::RelativeMismatch => "relative_mismatch",
            Self::Log10Ratio => "log10_ratio",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::RelativeMismatch => "PDF / |integrand| - 1",
            Self::Log10Ratio => "log10(PDF / |integrand|)",
        }
    }
}

fn selected_oversampling_metric(ctx: &TaskPanelContext<'_>) -> OversamplingMetric {
    match ctx.selected_value("pdf_adaptation_oversampling_metric") {
        Some("log10_ratio") => OversamplingMetric::Log10Ratio,
        _ => OversamplingMetric::RelativeMismatch,
    }
}

fn oversampling_metric_projector() -> TaskPanelProjector {
    let mut spec = panel_spec(
        "pdf_adaptation_oversampling_metric",
        "Sampling Accuracy Metric",
        PanelKind::Select,
        PanelHistoryMode::None,
    );
    spec.width = PanelWidth::Full;
    spec.state = Some(select_state_spec(
        JsonValue::String(OversamplingMetric::RelativeMismatch.as_str().to_string()),
        vec![
            state_option(
                OversamplingMetric::RelativeMismatch.as_str(),
                "Relative mismatch",
            ),
            state_option(OversamplingMetric::Log10Ratio.as_str(), "log10 ratio"),
        ],
        None,
    ));
    panel_projector_with_source(
        spec,
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        |_ctx| Ok(None),
        |_ctx| Ok(None),
    )
}

fn progress_projector(total: usize, label: &'static str, unit: &'static str) -> TaskPanelProjector {
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "pdf_adaptation_progress",
                label,
                PanelKind::Progress,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let processed = current_processed(ctx);
            Ok(Some(progress_panel(
                "pdf_adaptation_progress",
                processed as f64,
                Some(total as f64),
                Some(unit),
                None,
            )))
        },
        |_ctx| Ok(None),
    )
}

fn line_projector(
    panel_id: &'static str,
    label: &'static str,
    width: PanelWidth,
    geometry: LineRasterGeometry,
    image_kind: ImageKind,
) -> TaskPanelProjector {
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                panel_id,
                label,
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::None,
            ),
            width,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let Some(derived) = current_derived(ctx)? else {
                return Ok(None);
            };
            build_line_panel(
                panel_id,
                &geometry,
                &derived,
                image_kind,
                selected_oversampling_metric(ctx),
            )
            .map(Some)
        },
        |_ctx| Ok(None),
    )
}

fn image_projector(
    panel_id: &'static str,
    label: &'static str,
    width: PanelWidth,
    geometry: PlaneRasterGeometry,
    image_kind: ImageKind,
) -> TaskPanelProjector {
    panel_projector_with_source(
        with_panel_width(
            panel_spec(panel_id, label, PanelKind::Image2d, PanelHistoryMode::None),
            width,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let Some(derived) = current_derived(ctx)? else {
                return Ok(None);
            };
            build_image_panel(
                panel_id,
                &geometry,
                &derived,
                image_kind,
                selected_oversampling_metric(ctx),
            )
            .map(Some)
        },
        |_ctx| Ok(None),
    )
}

fn histogram_projector(
    panel_id: &'static str,
    label: &'static str,
    width: PanelWidth,
    image_kind: ImageKind,
) -> TaskPanelProjector {
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                panel_id,
                label,
                PanelKind::Histogram,
                PanelHistoryMode::None,
            ),
            width,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let Some(derived) = current_derived(ctx)? else {
                return Ok(None);
            };
            Ok(Some(histogram_panel(
                panel_id,
                &derived,
                image_kind,
                selected_oversampling_metric(ctx),
            )))
        },
        |_ctx| Ok(None),
    )
}

fn current_processed(ctx: &TaskPanelContext<'_>) -> usize {
    current_output(ctx)
        .ok()
        .flatten()
        .map(|output| output.processed)
        .unwrap_or_else(|| ctx.task.nr_completed_samples.max(0) as usize)
}

fn current_output(
    ctx: &TaskPanelContext<'_>,
) -> Result<Option<PdfAdaptationImagePersistedOutput>, EngineError> {
    match ctx.source.persisted() {
        Some(persisted) => serde_json::from_value(persisted.clone())
            .map(Some)
            .map_err(|err| {
                EngineError::build(format!("invalid pdf adaptation persisted output: {err}"))
            }),
        None => Ok(None),
    }
}

fn current_derived(ctx: &TaskPanelContext<'_>) -> Result<Option<DerivedValues>, EngineError> {
    Ok(current_output(ctx)?
        .map(|output| DerivedValues::from_output(output, target_abs_from_json(ctx.run_target))))
}

fn target_abs_from_json(run_target: Option<&JsonValue>) -> Option<f64> {
    let value = run_target?;
    if let Some(scalar) = value.as_f64() {
        return finite_positive_abs(scalar);
    }
    let object = value.as_object()?;
    let kind = object
        .get("kind")
        .or_else(|| object.get("type"))
        .and_then(JsonValue::as_str)
        .map(|value| value.to_ascii_lowercase());
    if matches!(kind.as_deref(), Some("scalar") | Some("value")) {
        return object
            .get("value")
            .and_then(JsonValue::as_f64)
            .and_then(finite_positive_abs);
    }
    let source = object
        .get("value")
        .and_then(JsonValue::as_object)
        .unwrap_or(object);
    let re = source
        .get("re")
        .or_else(|| source.get("real"))
        .and_then(JsonValue::as_f64)?;
    let im = source
        .get("im")
        .or_else(|| source.get("imag"))
        .and_then(JsonValue::as_f64)
        .unwrap_or(0.0);
    finite_positive_abs(re.hypot(im))
}

fn finite_positive_abs(value: f64) -> Option<f64> {
    let abs = value.abs();
    (abs.is_finite() && abs > 0.0).then_some(abs)
}

struct DerivedValues {
    output: PdfAdaptationImagePersistedOutput,
    reference_abs_integrand_norm: Option<f64>,
    log_plane_normalized_integrand: Vec<Option<f64>>,
    log_reference_normalized_integrand: Vec<Option<f64>>,
    log_plane_normalized_pdf: Vec<Option<f64>>,
    oversampling_legacy_ratio: Vec<Option<f64>>,
    oversampling_plane_normalized_ratio: Vec<Option<f64>>,
}

impl DerivedValues {
    fn from_output(
        output: PdfAdaptationImagePersistedOutput,
        target_abs_integrand_norm: Option<f64>,
    ) -> Self {
        let mean_abs_integrand = finite_mean(output.abs_integrand_values.iter().flatten().copied());
        let mean_pdf = finite_mean(output.pdf_values.iter().flatten().copied());
        let reference_abs_integrand_norm = target_abs_integrand_norm
            .or(output.global_abs_integrand_norm)
            .or(mean_abs_integrand);
        let log_plane_normalized_integrand = output
            .abs_integrand_values
            .iter()
            .map(|value| log10_ratio(*value, mean_abs_integrand))
            .collect::<Vec<_>>();
        let log_reference_normalized_integrand = output
            .abs_integrand_values
            .iter()
            .map(|value| log10_ratio(*value, reference_abs_integrand_norm))
            .collect::<Vec<_>>();
        let log_plane_normalized_pdf = output
            .pdf_values
            .iter()
            .map(|value| log10_ratio(*value, mean_pdf))
            .collect::<Vec<_>>();
        let oversampling_legacy_ratio = output
            .pdf_values
            .iter()
            .zip(output.abs_integrand_values.iter())
            .map(|(pdf, abs_integrand)| {
                plane_normalized_pdf_over_integrand_ratio(
                    *pdf,
                    *abs_integrand,
                    mean_abs_integrand,
                    mean_pdf,
                )
            })
            .collect::<Vec<_>>();
        let oversampling_plane_normalized_ratio = output
            .pdf_values
            .iter()
            .zip(output.abs_integrand_values.iter())
            .map(|(pdf, abs_integrand)| {
                pdf_over_integrand_global_norm_ratio(
                    *pdf,
                    *abs_integrand,
                    reference_abs_integrand_norm,
                    output.global_pdf_norm,
                )
            })
            .collect::<Vec<_>>();
        Self {
            output,
            reference_abs_integrand_norm,
            log_plane_normalized_integrand,
            log_reference_normalized_integrand,
            log_plane_normalized_pdf,
            oversampling_legacy_ratio,
            oversampling_plane_normalized_ratio,
        }
    }

    fn values(&self, image_kind: ImageKind, metric: OversamplingMetric) -> Vec<Option<f64>> {
        match image_kind {
            ImageKind::LogPlaneNormalizedIntegrand => {
                self.log_reference_normalized_integrand.clone()
            }
            ImageKind::LogPlaneNormalizedPdf => self.log_plane_normalized_pdf.clone(),
            ImageKind::OversamplingLegacy => {
                oversampling_values(&self.oversampling_legacy_ratio, metric)
            }
            ImageKind::OversamplingPlaneNormalized => {
                oversampling_values(&self.oversampling_plane_normalized_ratio, metric)
            }
        }
    }

    fn global_plane_oversampling_factor(&self) -> Option<(f64, usize)> {
        let i = self.reference_abs_integrand_norm?;
        let z = self.output.global_pdf_norm;
        if !i.is_finite() || i <= 0.0 || !z.is_finite() || z <= 0.0 {
            return None;
        }
        let mut pdf_mass_sum = 0.0;
        let mut integrand_mass_sum = 0.0;
        let mut count = 0usize;
        for (pdf, abs_integrand) in self
            .output
            .pdf_values
            .iter()
            .zip(self.output.abs_integrand_values.iter())
        {
            let (Some(pdf), Some(abs_integrand)) = (pdf, abs_integrand) else {
                continue;
            };
            if !pdf.is_finite() || !abs_integrand.is_finite() || *abs_integrand <= 0.0 {
                continue;
            }
            let pdf_mass = pdf / z;
            let integrand_mass = abs_integrand / i;
            if pdf_mass.is_finite()
                && integrand_mass.is_finite()
                && pdf_mass > 0.0
                && integrand_mass > 0.0
            {
                pdf_mass_sum += pdf_mass;
                integrand_mass_sum += integrand_mass;
                count += 1;
            }
        }
        if count == 0 || integrand_mass_sum <= 0.0 {
            return None;
        }
        Some((pdf_mass_sum / integrand_mass_sum, count))
    }
}

fn build_image_panel(
    panel_id: &str,
    geometry: &PlaneRasterGeometry,
    derived: &DerivedValues,
    image_kind: ImageKind,
    metric: OversamplingMetric,
) -> Result<PanelState, EngineError> {
    validate_output_length(geometry.nr_points(), &derived.output)?;
    let (values, invalid_indices) = option_values_to_image(&derived.values(image_kind, metric));
    Ok(PanelState::Image2d {
        panel_id: panel_id.to_string(),
        width: geometry.u_linspace.count,
        height: geometry.v_linspace.count,
        values,
        imag_values: None,
        invalid_indices,
        x_range: [geometry.u_linspace.start, geometry.u_linspace.stop],
        y_range: [geometry.v_linspace.start, geometry.v_linspace.stop],
        color_mode: ImageColorMode::ScalarHeatmap,
        normalization_mode: ImageNormalizationMode::Symmetric,
        metric_label: metric_label(image_kind, metric).map(str::to_string),
        metric_mode: metric_mode(image_kind, metric).map(str::to_string),
    })
}

fn build_line_panel(
    panel_id: &str,
    geometry: &LineRasterGeometry,
    derived: &DerivedValues,
    image_kind: ImageKind,
    metric: OversamplingMetric,
) -> Result<PanelState, EngineError> {
    validate_output_length(geometry.nr_points(), &derived.output)?;
    Ok(scalar_timeseries_panel(
        panel_id,
        line_points(geometry, &derived.values(image_kind, metric)),
    ))
}

fn histogram_panel(
    panel_id: &str,
    derived: &DerivedValues,
    image_kind: ImageKind,
    metric: OversamplingMetric,
) -> PanelState {
    let oversampling_legacy = derived.values(ImageKind::OversamplingLegacy, metric);
    let oversampling_plane_normalized =
        derived.values(ImageKind::OversamplingPlaneNormalized, metric);
    let bins = match image_kind {
        ImageKind::LogPlaneNormalizedIntegrand => {
            histogram_bins_on_shared_edges(
                &derived.log_plane_normalized_integrand,
                &derived.log_plane_normalized_pdf,
            )
            .0
        }
        ImageKind::LogPlaneNormalizedPdf => {
            histogram_bins_on_shared_edges(
                &derived.log_plane_normalized_integrand,
                &derived.log_plane_normalized_pdf,
            )
            .1
        }
        ImageKind::OversamplingLegacy => {
            histogram_bins_on_shared_edges(&oversampling_legacy, &oversampling_plane_normalized).0
        }
        ImageKind::OversamplingPlaneNormalized => {
            histogram_bins_on_shared_edges(&oversampling_legacy, &oversampling_plane_normalized).1
        }
    };
    PanelState::Histogram {
        panel_id: panel_id.to_string(),
        bins,
        controls: Some(pdf_adaptation_histogram_controls()),
    }
}

fn metric_label(image_kind: ImageKind, metric: OversamplingMetric) -> Option<&'static str> {
    match image_kind {
        ImageKind::OversamplingLegacy | ImageKind::OversamplingPlaneNormalized => {
            Some(metric.label())
        }
        ImageKind::LogPlaneNormalizedIntegrand => Some("log10(normalized integrand)"),
        ImageKind::LogPlaneNormalizedPdf => Some("log10(normalized PDF)"),
    }
}

fn metric_mode(image_kind: ImageKind, metric: OversamplingMetric) -> Option<&'static str> {
    match image_kind {
        ImageKind::OversamplingLegacy | ImageKind::OversamplingPlaneNormalized => {
            Some(metric.as_str())
        }
        ImageKind::LogPlaneNormalizedIntegrand => Some("log10_integrand"),
        ImageKind::LogPlaneNormalizedPdf => Some("log10_pdf"),
    }
}

fn pdf_adaptation_histogram_controls() -> serde_json::Value {
    json!({
        "scale": true,
        "x_scale": true,
        "pdf_cdf": true,
        "ratio": true,
        "relative_error": true,
        "export": true,
        "reset_view": true,
        "sort": true,
        "default_relative_error": false,
    })
}

fn validate_output_length(
    total: usize,
    output: &PdfAdaptationImagePersistedOutput,
) -> Result<(), EngineError> {
    if output.signed_integrand_values.len() != total
        || output.abs_integrand_values.len() != total
        || output.pdf_values.len() != total
    {
        return Err(EngineError::build(format!(
            "pdf adaptation payload length mismatch: expected {}, got signed={} integrand={} pdf={}",
            total,
            output.signed_integrand_values.len(),
            output.abs_integrand_values.len(),
            output.pdf_values.len(),
        )));
    }
    Ok(())
}

fn line_points(geometry: &LineRasterGeometry, values: &[Option<f64>]) -> Vec<PlotPoint> {
    (0..geometry.linspace.count)
        .filter_map(|index| {
            let y = values.get(index).copied().flatten()?;
            if !y.is_finite() {
                return None;
            }
            Some(PlotPoint {
                x: geometry.parameter_at(index),
                y,
                x_sampler_uptime_ms: None,
                x_completed_samples_total: None,
                y_min: None,
                y_max: None,
            })
        })
        .collect()
}

fn option_values_to_image(values: &[Option<f64>]) -> (Vec<f32>, Option<Vec<usize>>) {
    let mut image = Vec::with_capacity(values.len());
    let mut invalid_indices = Vec::new();
    for (index, value) in values.iter().enumerate() {
        match value {
            Some(value) if value.is_finite() => image.push(*value as f32),
            _ => {
                image.push(0.0);
                invalid_indices.push(index);
            }
        }
    }
    let invalid_indices = if invalid_indices.is_empty() {
        None
    } else {
        Some(invalid_indices)
    };
    (image, invalid_indices)
}

fn histogram_bins(values: &[Option<f64>]) -> Vec<HistogramBin> {
    let finite = values
        .iter()
        .flatten()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if finite.is_empty() {
        return Vec::new();
    }
    let min = finite.iter().copied().fold(f64::INFINITY, f64::min);
    let max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if min == max {
        return vec![HistogramBin {
            start: min - 0.5,
            stop: max + 0.5,
            value: 1.0,
            error: 0.0,
        }];
    }
    let width = (max - min) / HISTOGRAM_BIN_COUNT as f64;
    let mut counts = vec![0usize; HISTOGRAM_BIN_COUNT];
    for value in finite.iter().copied() {
        let mut index = ((value - min) / width).floor() as usize;
        if index >= HISTOGRAM_BIN_COUNT {
            index = HISTOGRAM_BIN_COUNT - 1;
        }
        counts[index] += 1;
    }
    let total = finite.len() as f64;
    counts
        .into_iter()
        .enumerate()
        .map(|(index, count)| {
            let start = min + index as f64 * width;
            let stop = if index + 1 == HISTOGRAM_BIN_COUNT {
                max
            } else {
                start + width
            };
            let value = if width > 0.0 {
                count as f64 / (total * width)
            } else {
                0.0
            };
            let error = if width > 0.0 {
                (count as f64).sqrt() / (total * width)
            } else {
                0.0
            };
            HistogramBin {
                start,
                stop,
                value,
                error,
            }
        })
        .collect()
}

fn histogram_bins_on_shared_edges(
    left: &[Option<f64>],
    right: &[Option<f64>],
) -> (Vec<HistogramBin>, Vec<HistogramBin>) {
    let left_finite = left
        .iter()
        .flatten()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let right_finite = right
        .iter()
        .flatten()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if left_finite.is_empty() || right_finite.is_empty() {
        return (histogram_bins(left), histogram_bins(right));
    }
    let min = left_finite
        .iter()
        .chain(right_finite.iter())
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max = left_finite
        .iter()
        .chain(right_finite.iter())
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    if !min.is_finite() || !max.is_finite() || min == max {
        return (histogram_bins(left), histogram_bins(right));
    }
    let width = (max - min) / HISTOGRAM_BIN_COUNT as f64;
    if !width.is_finite() || width <= 0.0 {
        return (histogram_bins(left), histogram_bins(right));
    }
    let left_bins = histogram_bins_with_fixed_edges(&left_finite, min, max, HISTOGRAM_BIN_COUNT);
    let right_bins = histogram_bins_with_fixed_edges(&right_finite, min, max, HISTOGRAM_BIN_COUNT);
    (left_bins, right_bins)
}

fn histogram_bins_with_fixed_edges(
    finite: &[f64],
    min: f64,
    max: f64,
    bin_count: usize,
) -> Vec<HistogramBin> {
    if finite.is_empty() || bin_count == 0 || !min.is_finite() || !max.is_finite() || min == max {
        return Vec::new();
    }
    let width = (max - min) / bin_count as f64;
    if !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }
    let mut counts = vec![0usize; bin_count];
    for value in finite.iter().copied() {
        let mut index = ((value - min) / width).floor() as usize;
        if index >= bin_count {
            index = bin_count - 1;
        }
        counts[index] += 1;
    }
    let total = finite.len() as f64;
    counts
        .into_iter()
        .enumerate()
        .map(|(index, count)| {
            let start = min + index as f64 * width;
            let stop = if index + 1 == bin_count {
                max
            } else {
                start + width
            };
            let value = count as f64 / (total * width);
            let error = (count as f64).sqrt() / (total * width);
            HistogramBin {
                start,
                stop,
                value,
                error,
            }
        })
        .collect()
}

fn log10_ratio(value: Option<f64>, mean: Option<f64>) -> Option<f64> {
    match (value, mean) {
        (Some(value), Some(scale)) if value > 0.0 && scale > 0.0 => Some((value / scale).log10()),
        _ => None,
    }
}

fn oversampling_values(ratios: &[Option<f64>], metric: OversamplingMetric) -> Vec<Option<f64>> {
    ratios
        .iter()
        .map(|ratio| match (ratio, metric) {
            (Some(ratio), OversamplingMetric::RelativeMismatch) if ratio.is_finite() => {
                Some(ratio - 1.0)
            }
            (Some(ratio), OversamplingMetric::Log10Ratio) if ratio.is_finite() && *ratio > 0.0 => {
                Some(ratio.log10())
            }
            _ => None,
        })
        .collect()
}

fn plane_normalized_pdf_over_integrand_ratio(
    pdf: Option<f64>,
    abs_integrand: Option<f64>,
    mean_abs_integrand: Option<f64>,
    mean_pdf: Option<f64>,
) -> Option<f64> {
    match (pdf, abs_integrand, mean_abs_integrand, mean_pdf) {
        (Some(pdf), Some(abs_integrand), Some(i), Some(z))
            if pdf.is_finite()
                && abs_integrand.is_finite()
                && i.is_finite()
                && z.is_finite()
                && abs_integrand > 0.0
                && i > 0.0
                && z > 0.0 =>
        {
            let ratio = (pdf / z) / (abs_integrand / i);
            ratio.is_finite().then_some(ratio)
        }
        _ => None,
    }
}

fn pdf_over_integrand_global_norm_ratio(
    pdf: Option<f64>,
    abs_integrand: Option<f64>,
    global_abs_integrand_norm: Option<f64>,
    global_pdf_norm: f64,
) -> Option<f64> {
    match (pdf, abs_integrand, global_abs_integrand_norm) {
        (Some(pdf), Some(abs_integrand), Some(i))
            if pdf > 0.0
                && abs_integrand > 0.0
                && i > 0.0
                && global_pdf_norm.is_finite()
                && global_pdf_norm > 0.0 =>
        {
            let ratio = (pdf / global_pdf_norm) / (abs_integrand / i);
            ratio.is_finite().then_some(ratio)
        }
        _ => None,
    }
}

fn finite_mean(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut sum = 0.0;
    let mut count = 0usize;
    for value in values {
        if value.is_finite() {
            sum += value;
            count += 1;
        }
    }
    (count > 0).then_some(sum / count as f64)
}

#[cfg(test)]
mod tests {
    use super::{
        ImageKind, OversamplingMetric, build_image_panel, build_line_panel, histogram_bins,
        oversampling_values,
    };
    use crate::core::{LineRasterGeometry, Linspace, PlaneRasterGeometry};
    use crate::sampling::PdfAdaptationImagePersistedOutput;
    use crate::server::panels::{ImageNormalizationMode, PanelState};
    use crate::server::task_panels::pdf_adaptation::DerivedValues;

    fn geometry() -> PlaneRasterGeometry {
        PlaneRasterGeometry {
            offset: vec![0.0, 0.0],
            u_vector: vec![1.0, 0.0],
            v_vector: vec![0.0, 1.0],
            u_linspace: Linspace {
                start: 0.0,
                stop: 1.0,
                count: 2,
            },
            v_linspace: Linspace {
                start: 0.0,
                stop: 1.0,
                count: 1,
            },
            discrete: vec![],
        }
    }

    fn output() -> PdfAdaptationImagePersistedOutput {
        PdfAdaptationImagePersistedOutput {
            processed: 2,
            abs_integrand_mean: Some(3.0),
            global_abs_integrand_norm: Some(5.0),
            global_pdf_norm: 1.0,
            signed_integrand_values: vec![Some(-2.0), Some(4.0)],
            abs_integrand_values: vec![Some(2.0), Some(4.0)],
            pdf_values: vec![Some(1.0), Some(2.0)],
        }
    }

    #[test]
    fn global_plane_oversampling_factor_is_computed_from_global_norms() {
        let derived = DerivedValues::from_output(output(), None);
        let (factor, count) = derived
            .global_plane_oversampling_factor()
            .expect("global oversampling factor");
        assert_eq!(count, 2);
        let expected = ((1.0 / 1.0) + (2.0 / 1.0)) / ((2.0 / 5.0) + (4.0 / 5.0));
        assert!((factor - expected).abs() < 1e-12);
    }

    fn line_geometry() -> LineRasterGeometry {
        LineRasterGeometry {
            offset: vec![0.0, 0.0],
            direction: vec![1.0, 0.0],
            linspace: Linspace {
                start: 0.0,
                stop: 1.0,
                count: 2,
            },
            discrete: vec![],
        }
    }

    #[test]
    fn derived_values_compute_log_panels() {
        let derived = DerivedValues::from_output(output(), None);
        assert_eq!(
            derived.log_plane_normalized_integrand,
            vec![Some((2.0_f64 / 3.0).log10()), Some((4.0_f64 / 3.0).log10())]
        );
        assert_eq!(
            derived.log_plane_normalized_pdf,
            vec![Some((1.0_f64 / 1.5).log10()), Some((2.0_f64 / 1.5).log10())]
        );
        assert_eq!(
            derived.oversampling_legacy_ratio,
            vec![Some(1.0), Some(1.0)]
        );
        assert_eq!(
            derived.oversampling_plane_normalized_ratio,
            vec![Some(2.5), Some(2.5)]
        );
        assert_eq!(
            oversampling_values(
                &derived.oversampling_plane_normalized_ratio,
                OversamplingMetric::RelativeMismatch,
            ),
            vec![Some(1.5), Some(1.5)]
        );
        assert_eq!(
            oversampling_values(
                &derived.oversampling_plane_normalized_ratio,
                OversamplingMetric::Log10Ratio,
            ),
            vec![Some(2.5_f64.log10()), Some(2.5_f64.log10())]
        );
    }

    #[test]
    fn plane_normalized_pdf_panel_uses_symmetric_normalization() {
        let panel = build_image_panel(
            "pdf",
            &geometry(),
            &DerivedValues::from_output(output(), None),
            ImageKind::LogPlaneNormalizedPdf,
            OversamplingMetric::RelativeMismatch,
        )
        .expect("build plane normalized pdf panel");
        let PanelState::Image2d {
            normalization_mode, ..
        } = panel
        else {
            panic!("expected image panel");
        };
        assert!(matches!(
            normalization_mode,
            ImageNormalizationMode::Symmetric
        ));
    }

    #[test]
    fn oversampling_panel_uses_symmetric_normalization() {
        let panel = build_image_panel(
            "oversampling",
            &geometry(),
            &DerivedValues::from_output(output(), None),
            ImageKind::OversamplingLegacy,
            OversamplingMetric::RelativeMismatch,
        )
        .expect("build oversampling panel");
        let PanelState::Image2d {
            normalization_mode, ..
        } = panel
        else {
            panic!("expected image panel");
        };
        assert!(matches!(
            normalization_mode,
            ImageNormalizationMode::Symmetric
        ));
    }

    #[test]
    fn histogram_bins_cover_finite_values() {
        let bins = histogram_bins(&[Some(-1.0), Some(0.0), Some(1.0), None]);
        assert!(!bins.is_empty());
        let total_mass = bins
            .iter()
            .map(|bin| (bin.stop - bin.start) * bin.value)
            .sum::<f64>();
        assert!((total_mass - 1.0).abs() < 1e-9);
    }

    #[test]
    fn line_panel_projects_log_pdf_metric() {
        let panel = build_line_panel(
            "pdf_adaptation_log_pdf_line",
            &line_geometry(),
            &DerivedValues::from_output(output(), None),
            ImageKind::LogPlaneNormalizedPdf,
            OversamplingMetric::RelativeMismatch,
        )
        .expect("build line panel");
        let PanelState::ScalarTimeseries { points, .. } = panel else {
            panic!("expected scalar timeseries");
        };
        assert_eq!(points.len(), 2);
        assert!((points[0].x - 0.0).abs() < 1e-12);
        assert!((points[1].x - 1.0).abs() < 1e-12);
        assert!((points[0].y - (1.0_f64 / 1.5).log10()).abs() < 1e-12);
        assert!((points[1].y - (2.0_f64 / 1.5).log10()).abs() < 1e-12);
    }
}
