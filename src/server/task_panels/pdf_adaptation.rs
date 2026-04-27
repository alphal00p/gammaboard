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
    scalar_timeseries_panel, with_panel_width,
};

const HISTOGRAM_BIN_COUNT: usize = 31;

pub(super) fn projectors(
    geometry: PlaneRasterGeometry,
    _display: ImageDisplayMode,
) -> Vec<TaskPanelProjector> {
    vec![
        progress_projector(geometry.nr_points(), "Image Progress", "pixels"),
        completion_projector(geometry.nr_points(), "Image Completion"),
        summary_projector(),
        image_projector(
            "pdf_adaptation_log_integrand",
            "Normalized integrand",
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
            "Histogram: Normalized integrand",
            PanelWidth::Half,
            ImageKind::LogPlaneNormalizedIntegrand,
        ),
        histogram_projector(
            "pdf_adaptation_log_pdf_histogram",
            "Histogram: Normalized PDF",
            PanelWidth::Half,
            ImageKind::LogPlaneNormalizedPdf,
        ),
        image_projector(
            "pdf_adaptation_oversampling",
            "Sampling Accuracy",
            PanelWidth::Half,
            geometry.clone(),
            ImageKind::OversamplingLegacy,
        ),
        image_projector(
            "pdf_adaptation_oversampling_plane_normalized",
            "Normalized Sampling Accuracy",
            PanelWidth::Half,
            geometry,
            ImageKind::OversamplingPlaneNormalized,
        ),
        histogram_projector(
            "pdf_adaptation_oversampling_histogram",
            "Histogram: Sampling Accuracy",
            PanelWidth::Half,
            ImageKind::OversamplingLegacy,
        ),
        histogram_projector(
            "pdf_adaptation_oversampling_plane_normalized_histogram",
            "Histogram: Normalized Sampling Accuracy",
            PanelWidth::Half,
            ImageKind::OversamplingPlaneNormalized,
        ),
    ]
}

pub(super) fn line_projectors(
    geometry: LineRasterGeometry,
    _display: LineDisplayMode,
) -> Vec<TaskPanelProjector> {
    vec![
        progress_projector(geometry.nr_points(), "Line Progress", "points"),
        completion_projector(geometry.nr_points(), "Line Completion"),
        summary_projector(),
        line_projector(
            "pdf_adaptation_log_integrand_line",
            "Normalized integrand (1D)",
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
        line_projector(
            "pdf_adaptation_oversampling_line",
            "Sampling Accuracy (1D)",
            PanelWidth::Half,
            geometry.clone(),
            ImageKind::OversamplingLegacy,
        ),
        line_projector(
            "pdf_adaptation_oversampling_plane_normalized_line",
            "Normalized Sampling Accuracy (1D)",
            PanelWidth::Half,
            geometry,
            ImageKind::OversamplingPlaneNormalized,
        ),
        histogram_projector(
            "pdf_adaptation_log_integrand_histogram",
            "Histogram: Normalized integrand",
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
            "Histogram: Sampling Accuracy",
            PanelWidth::Half,
            ImageKind::OversamplingLegacy,
        ),
        histogram_projector(
            "pdf_adaptation_oversampling_plane_normalized_histogram",
            "Histogram: Normalized Sampling Accuracy",
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

fn completion_projector(total: usize, label: &'static str) -> TaskPanelProjector {
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "pdf_adaptation_completion",
                label,
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Compact,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let processed = current_processed(ctx);
            Ok(Some(key_value_panel(
                "pdf_adaptation_completion",
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
            let Some(derived) = current_output(ctx)?.map(DerivedValues::from_output) else {
                return Ok(None);
            };
            build_line_panel(panel_id, &geometry, &derived, image_kind).map(Some)
        },
        |_ctx| Ok(None),
    )
}

fn summary_projector() -> TaskPanelProjector {
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "pdf_adaptation_summary",
                "Adaptation Summary",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        |ctx| {
            let Some(derived) = current_output(ctx)?.map(DerivedValues::from_output) else {
                return Ok(None);
            };
            let valid_ratio_count = derived
                .oversampling_legacy_log10
                .iter()
                .filter(|value| value.is_some())
                .count();
            let mean_ratio = finite_mean(
                derived
                    .oversampling_legacy_log10
                    .iter()
                    .filter_map(|value| value.map(|v| 10_f64.powf(-v))),
            );
            Ok(Some(key_value_panel(
                "pdf_adaptation_summary",
                vec![
                    key_value("processed", "Processed", derived.output.processed),
                    key_value(
                        "abs_integrand_mean",
                        "Mean |Integrand|",
                        derived.output.abs_integrand_mean,
                    ),
                    key_value(
                        "pdf_defined_points",
                        "PDF Defined",
                        derived
                            .output
                            .pdf_values
                            .iter()
                            .filter(|value| value.is_some())
                            .count(),
                    ),
                    key_value("valid_ratio_count", "Valid Oversampling", valid_ratio_count),
                    key_value("mean_inverse_ratio", "Mean((|I| / <|I|>) / P)", mean_ratio),
                ],
            )))
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
            let Some(derived) = current_output(ctx)?.map(DerivedValues::from_output) else {
                return Ok(None);
            };
            build_image_panel(panel_id, &geometry, &derived, image_kind).map(Some)
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
            let Some(derived) = current_output(ctx)?.map(DerivedValues::from_output) else {
                return Ok(None);
            };
            Ok(Some(histogram_panel(panel_id, &derived, image_kind)))
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

struct DerivedValues {
    output: PdfAdaptationImagePersistedOutput,
    log_plane_normalized_integrand: Vec<Option<f64>>,
    log_plane_normalized_pdf: Vec<Option<f64>>,
    oversampling_legacy_log10: Vec<Option<f64>>,
    oversampling_plane_normalized_log10: Vec<Option<f64>>,
}

impl DerivedValues {
    fn from_output(output: PdfAdaptationImagePersistedOutput) -> Self {
        let mean_abs_integrand = finite_mean(output.abs_integrand_values.iter().flatten().copied());
        let mean_pdf = finite_mean(output.pdf_values.iter().flatten().copied());
        let log_plane_normalized_integrand = output
            .abs_integrand_values
            .iter()
            .map(|value| log10_ratio(*value, mean_abs_integrand))
            .collect::<Vec<_>>();
        let log_plane_normalized_pdf = output
            .pdf_values
            .iter()
            .map(|value| log10_ratio(*value, mean_pdf))
            .collect::<Vec<_>>();
        let oversampling_legacy_log10 = output
            .pdf_values
            .iter()
            .zip(output.abs_integrand_values.iter())
            .map(|(pdf, abs_integrand)| {
                log10_pdf_over_integrand_over_mean(*pdf, *abs_integrand, output.abs_integrand_mean)
            })
            .collect::<Vec<_>>();
        let oversampling_plane_normalized_log10 = log_plane_normalized_pdf
            .iter()
            .zip(log_plane_normalized_integrand.iter())
            .map(|(pdf, integrand)| match (pdf, integrand) {
                (Some(pdf), Some(integrand)) => Some(pdf - integrand),
                _ => None,
            })
            .collect::<Vec<_>>();
        Self {
            output,
            log_plane_normalized_integrand,
            log_plane_normalized_pdf,
            oversampling_legacy_log10,
            oversampling_plane_normalized_log10,
        }
    }

    fn values(&self, image_kind: ImageKind) -> &[Option<f64>] {
        match image_kind {
            ImageKind::LogPlaneNormalizedIntegrand => &self.log_plane_normalized_integrand,
            ImageKind::LogPlaneNormalizedPdf => &self.log_plane_normalized_pdf,
            ImageKind::OversamplingLegacy => &self.oversampling_legacy_log10,
            ImageKind::OversamplingPlaneNormalized => &self.oversampling_plane_normalized_log10,
        }
    }
}

fn build_image_panel(
    panel_id: &str,
    geometry: &PlaneRasterGeometry,
    derived: &DerivedValues,
    image_kind: ImageKind,
) -> Result<PanelState, EngineError> {
    validate_output_length(geometry.nr_points(), &derived.output)?;
    let (values, invalid_indices) = option_values_to_image(derived.values(image_kind));
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
    })
}

fn build_line_panel(
    panel_id: &str,
    geometry: &LineRasterGeometry,
    derived: &DerivedValues,
    image_kind: ImageKind,
) -> Result<PanelState, EngineError> {
    validate_output_length(geometry.nr_points(), &derived.output)?;
    Ok(scalar_timeseries_panel(
        panel_id,
        line_points(geometry, derived.values(image_kind)),
    ))
}

fn histogram_panel(panel_id: &str, derived: &DerivedValues, image_kind: ImageKind) -> PanelState {
    PanelState::Histogram {
        panel_id: panel_id.to_string(),
        bins: histogram_bins(derived.values(image_kind)),
    }
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
                x: line_x_value(geometry, index),
                y,
                x_sampler_uptime_ms: None,
                x_completed_samples_total: None,
                y_min: None,
                y_max: None,
            })
        })
        .collect()
}

fn line_x_value(geometry: &LineRasterGeometry, index: usize) -> f64 {
    if geometry.linspace.count <= 1 {
        return geometry.linspace.start;
    }
    let t = index as f64 / (geometry.linspace.count - 1) as f64;
    geometry.linspace.start + t * (geometry.linspace.stop - geometry.linspace.start)
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
            let value = count as f64 / total;
            HistogramBin {
                start,
                stop,
                value,
                error: value.sqrt() / total.sqrt(),
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

fn log10_pdf_over_integrand_over_mean(
    pdf: Option<f64>,
    abs_integrand: Option<f64>,
    abs_integrand_mean: Option<f64>,
) -> Option<f64> {
    match (pdf, abs_integrand, abs_integrand_mean) {
        (Some(pdf), Some(abs_integrand), Some(mean))
            if pdf > 0.0 && abs_integrand > 0.0 && mean > 0.0 =>
        {
            Some((pdf / (abs_integrand / mean)).log10())
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
    use super::{ImageKind, build_image_panel, build_line_panel, histogram_bins};
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
            signed_integrand_values: vec![Some(-2.0), Some(4.0)],
            abs_integrand_values: vec![Some(2.0), Some(4.0)],
            pdf_values: vec![Some(1.0), Some(2.0)],
        }
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
        let derived = DerivedValues::from_output(output());
        assert_eq!(
            derived.log_plane_normalized_integrand,
            vec![Some((2.0_f64 / 3.0).log10()), Some((4.0_f64 / 3.0).log10())]
        );
        assert_eq!(
            derived.log_plane_normalized_pdf,
            vec![Some((1.0_f64 / 1.5).log10()), Some((2.0_f64 / 1.5).log10())]
        );
        assert_eq!(
            derived.oversampling_legacy_log10,
            vec![
                Some((1.0_f64 / (2.0 / 3.0)).log10()),
                Some((2.0_f64 / (4.0 / 3.0)).log10())
            ]
        );
        assert_eq!(
            derived.oversampling_plane_normalized_log10,
            vec![Some(0.0), Some(0.0)]
        );
    }

    #[test]
    fn plane_normalized_pdf_panel_uses_symmetric_normalization() {
        let panel = build_image_panel(
            "pdf",
            &geometry(),
            &DerivedValues::from_output(output()),
            ImageKind::LogPlaneNormalizedPdf,
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
            &DerivedValues::from_output(output()),
            ImageKind::OversamplingLegacy,
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
        let total_density = bins.iter().map(|bin| bin.value).sum::<f64>();
        assert!((total_density - 1.0).abs() < 1e-9);
    }

    #[test]
    fn line_panel_projects_log_pdf_metric() {
        let panel = build_line_panel(
            "pdf_adaptation_log_pdf_line",
            &line_geometry(),
            &DerivedValues::from_output(output()),
            ImageKind::LogPlaneNormalizedPdf,
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
