use super::{TaskPanelContext, TaskPanelHistoryContext, TaskPanelProjector, panel_projector};
use crate::core::{EngineError, ObservableConfig, RunTaskSpec};
use crate::evaluation::{
    FullObservableProgress, GammaLoopObservableState, Observable, ObservableState,
    SemanticObservableKind,
};
use crate::server::panels::{
    HistogramBin, PanelHistoryMode, PanelKind, PanelState, PanelWidth, PlotPoint, key_value,
    key_value_panel, panel_spec, progress_panel, scalar_timeseries_panel, single_point_band,
    table_panel_with_payload, with_panel_width,
};
use gammalooprs::observables::{ObservablePhase, ObservableValueTransform};
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

pub(super) fn projectors(
    task_spec: &RunTaskSpec,
    effective_observable_config: Option<ObservableConfig>,
) -> Vec<TaskPanelProjector> {
    let observable_config = task_observable_config(task_spec).or(effective_observable_config);
    let mut projectors = vec![
        sample_progress_projector(),
        estimate_summary_projector(observable_config.as_ref()),
        real_estimate_history_projector(observable_config.as_ref()),
    ];
    if matches!(
        observable_config,
        Some(ObservableConfig::Complex | ObservableConfig::Gammaloop)
    ) {
        projectors.push(imag_estimate_history_projector(observable_config.as_ref()));
    }
    projectors.push(abs_signal_to_noise_history_projector(
        observable_config.as_ref(),
    ));
    if matches!(observable_config, Some(ObservableConfig::Gammaloop)) {
        projectors.push(gammaloop_histogram_bundle_projector());
    }
    projectors
}

fn sample_progress_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                "sample_progress",
                "Sample Progress",
                PanelKind::Progress,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            let current = sample_progress_value(ctx)?;
            Ok(Some(progress_panel(
                "sample_progress",
                current,
                ctx.task
                    .task
                    .nr_expected_samples()
                    .map(|value| value as f64),
                Some("samples"),
            )))
        },
        |_ctx| Ok(None),
    )
}

fn real_estimate_history_projector(
    observable_config: Option<&ObservableConfig>,
) -> TaskPanelProjector {
    let observable_config = observable_config.cloned();
    let current_config = observable_config.clone();
    let history_config = observable_config.clone();
    panel_projector(
        with_panel_width(
            panel_spec(
                "real_estimate_history",
                estimate_label(observable_config.as_ref()),
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        move |ctx| {
            Ok(sample_observable(ctx, current_config.as_ref())?.map(real_estimate_history_panel))
        },
        move |ctx| {
            Ok(decode_history_observable(ctx, history_config.as_ref())?
                .map(real_estimate_history_panel))
        },
    )
}

fn imag_estimate_history_projector(
    observable_config: Option<&ObservableConfig>,
) -> TaskPanelProjector {
    let observable_config = observable_config.cloned();
    let current_config = observable_config.clone();
    let history_config = observable_config.clone();
    panel_projector(
        with_panel_width(
            panel_spec(
                "imag_estimate_history",
                "Imaginary Mean",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        move |ctx| {
            Ok(sample_observable(ctx, current_config.as_ref())?
                .and_then(imag_estimate_history_panel))
        },
        move |ctx| {
            Ok(decode_history_observable(ctx, history_config.as_ref())?
                .and_then(imag_estimate_history_panel))
        },
    )
}

fn abs_signal_to_noise_history_projector(
    observable_config: Option<&ObservableConfig>,
) -> TaskPanelProjector {
    let observable_config = observable_config.cloned();
    let current_config = observable_config.clone();
    let history_config = observable_config.clone();
    panel_projector(
        with_panel_width(
            panel_spec(
                "abs_signal_to_noise_history",
                "Mean(|x|)^2 / abs_err^2",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        move |ctx| {
            Ok(sample_observable(ctx, current_config.as_ref())?.map(abs_signal_to_noise_panel))
        },
        move |ctx| {
            Ok(decode_history_observable(ctx, history_config.as_ref())?
                .map(abs_signal_to_noise_panel))
        },
    )
}

fn estimate_summary_projector(observable_config: Option<&ObservableConfig>) -> TaskPanelProjector {
    let observable_config = observable_config.cloned();
    panel_projector(
        with_panel_width(
            panel_spec(
                "estimate_summary",
                "Estimate Summary",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        move |ctx| {
            Ok(sample_observable(ctx, observable_config.as_ref())?.map(estimate_summary_panel))
        },
        |_ctx| Ok(None),
    )
}

fn gammaloop_histogram_bundle_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                "gammaloop_histogram_bundle",
                "Histogram Bundle",
                PanelKind::Table,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            Ok(sample_observable(ctx, Some(&ObservableConfig::Gammaloop))?
                .and_then(gammaloop_histogram_bundle_panel))
        },
        |ctx| {
            Ok(
                decode_history_observable(ctx, Some(&ObservableConfig::Gammaloop))?
                    .and_then(gammaloop_histogram_bundle_panel),
            )
        },
    )
}

fn sample_progress_value(ctx: &TaskPanelContext<'_>) -> Result<f64, EngineError> {
    if let Some(persisted) = ctx.source.persisted() {
        if let Some(observable) = decode_aggregate_persisted_observable_with_fallback(
            task_observable_config(&ctx.task.task).as_ref(),
            persisted,
        )? {
            return Ok(observable.sample_count() as f64);
        }
        if let Ok(progress) = decode_full_progress(persisted) {
            return Ok(progress.processed as f64);
        }
    }
    Ok(ctx.task.nr_completed_samples.max(0) as f64)
}

fn sample_observable(
    ctx: &TaskPanelContext<'_>,
    observable_config: Option<&ObservableConfig>,
) -> Result<Option<ObservableState>, EngineError> {
    if let Some(observable) = ctx.source.observable() {
        return Ok(Some(observable.clone()));
    }
    match ctx.source.persisted() {
        Some(persisted) => {
            decode_aggregate_persisted_observable_with_fallback(observable_config, persisted)
        }
        None => Ok(None),
    }
}

fn decode_history_observable(
    ctx: &TaskPanelHistoryContext<'_>,
    observable_config: Option<&ObservableConfig>,
) -> Result<Option<ObservableState>, EngineError> {
    decode_aggregate_persisted_observable_with_fallback(
        observable_config,
        &ctx.snapshot.persisted_output,
    )
}

fn decode_aggregate_persisted_observable(
    config: &ObservableConfig,
    persisted: &JsonValue,
) -> Result<ObservableState, EngineError> {
    match config {
        ObservableConfig::Scalar => ObservableState::from_aggregate_persistent_json(
            SemanticObservableKind::Scalar,
            persisted,
        ),
        ObservableConfig::Complex => ObservableState::from_aggregate_persistent_json(
            SemanticObservableKind::Complex,
            persisted,
        ),
        ObservableConfig::Gammaloop => ObservableState::from_gammaloop_persistent_json(persisted),
        ObservableConfig::FullScalar | ObservableConfig::FullComplex => {
            Err(EngineError::build(format!(
                "sample task expected aggregate observable, got {}",
                config_label(config)
            )))
        }
    }
}

fn decode_full_progress(persisted: &JsonValue) -> Result<FullObservableProgress, EngineError> {
    serde_json::from_value(persisted.clone())
        .map_err(|err| EngineError::build(format!("invalid full observable progress: {err}")))
}

fn decode_aggregate_persisted_observable_with_fallback(
    observable_config: Option<&ObservableConfig>,
    persisted: &JsonValue,
) -> Result<Option<ObservableState>, EngineError> {
    if let Some(config) = observable_config {
        return decode_aggregate_persisted_observable(config, persisted).map(Some);
    }
    if let Ok(observable) =
        decode_aggregate_persisted_observable(&ObservableConfig::Scalar, persisted)
    {
        return Ok(Some(observable));
    }
    if let Ok(observable) =
        decode_aggregate_persisted_observable(&ObservableConfig::Complex, persisted)
    {
        return Ok(Some(observable));
    }
    if let Ok(observable) =
        decode_aggregate_persisted_observable(&ObservableConfig::Gammaloop, persisted)
    {
        return Ok(Some(observable));
    }
    Ok(None)
}

fn estimate_label(observable_config: Option<&ObservableConfig>) -> &'static str {
    match observable_config {
        Some(ObservableConfig::Scalar) => "Mean",
        Some(ObservableConfig::Complex) => "Real Mean",
        Some(ObservableConfig::Gammaloop) => "Real Mean",
        None => "Estimate",
        Some(ObservableConfig::FullScalar) | Some(ObservableConfig::FullComplex) => "Estimate",
    }
}

fn task_observable_config(task: &RunTaskSpec) -> Option<ObservableConfig> {
    task.new_observable_config().ok().flatten()
}

fn config_label(config: &ObservableConfig) -> &'static str {
    match config {
        ObservableConfig::Scalar => "scalar",
        ObservableConfig::Complex => "complex",
        ObservableConfig::Gammaloop => "gammaloop",
        ObservableConfig::FullScalar => "full_scalar",
        ObservableConfig::FullComplex => "full_complex",
    }
}

fn real_estimate_history_panel(observable: ObservableState) -> PanelState {
    match observable {
        ObservableState::Scalar(state) => single_point_band(
            "real_estimate_history",
            state.count as f64,
            state.mean(),
            Some(state.mean() - state.stderr()),
            Some(state.mean() + state.stderr()),
        ),
        ObservableState::Complex(state) => single_point_band(
            "real_estimate_history",
            state.count as f64,
            state.real_mean(),
            Some(state.real_mean() - state.real_stderr()),
            Some(state.real_mean() + state.real_stderr()),
        ),
        ObservableState::Gammaloop(state) => single_point_band(
            "real_estimate_history",
            state.sample_count() as f64,
            state.real_mean(),
            Some(state.real_mean() - state.real_stderr()),
            Some(state.real_mean() + state.real_stderr()),
        ),
        _ => scalar_timeseries_panel("real_estimate_history", Vec::new()),
    }
}

fn imag_estimate_history_panel(observable: ObservableState) -> Option<PanelState> {
    match observable {
        ObservableState::Complex(state) => Some(single_point_band(
            "imag_estimate_history",
            state.count as f64,
            state.imag_mean(),
            Some(state.imag_mean() - state.imag_stderr()),
            Some(state.imag_mean() + state.imag_stderr()),
        )),
        ObservableState::Gammaloop(state) => Some(single_point_band(
            "imag_estimate_history",
            state.sample_count() as f64,
            state.imag_mean(),
            Some(state.imag_mean() - state.imag_stderr()),
            Some(state.imag_mean() + state.imag_stderr()),
        )),
        _ => None,
    }
}

fn abs_signal_to_noise_panel(observable: ObservableState) -> PanelState {
    scalar_timeseries_panel(
        "abs_signal_to_noise_history",
        vec![PlotPoint {
            x: observable.sample_count() as f64,
            y: observable.abs_signal_to_noise(),
            y_min: None,
            y_max: None,
        }],
    )
}

fn estimate_summary_panel(observable: ObservableState) -> PanelState {
    match observable {
        ObservableState::Scalar(state) => key_value_panel(
            "estimate_summary",
            vec![
                key_value("count", "Count", state.count),
                key_value(
                    "mean",
                    "Mean",
                    format_estimate_with_error(state.mean(), state.stderr()),
                ),
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
            "estimate_summary",
            vec![
                key_value("count", "Count", state.count),
                key_value(
                    "real_mean",
                    "Real Mean",
                    format_estimate_with_error(state.real_mean(), state.real_stderr()),
                ),
                key_value(
                    "imag_mean",
                    "Imag Mean",
                    format_estimate_with_error(state.imag_mean(), state.imag_stderr()),
                ),
                key_value(
                    "abs_mean",
                    "Abs Mean",
                    format_estimate_with_error(state.abs_mean(), state.abs_stderr()),
                ),
                key_value(
                    "signal_to_noise",
                    "Mean(|x|)^2 / abs_err^2",
                    state.signal_to_noise(),
                ),
                key_value("rsd", "RSD", state.rsd()),
            ],
        ),
        ObservableState::Gammaloop(state) => key_value_panel(
            "estimate_summary",
            vec![
                key_value("count", "Count", state.sample_count()),
                key_value("histograms", "Histograms", state.histogram_count()),
                key_value(
                    "primary_histogram",
                    "Primary Histogram",
                    state.primary_histogram_name().unwrap_or("-"),
                ),
                key_value(
                    "primary_title",
                    "Primary Title",
                    state
                        .primary_histogram()
                        .map(|histogram| histogram.title.as_str())
                        .unwrap_or("-"),
                ),
                key_value(
                    "primary_samples",
                    "Primary Samples",
                    state
                        .primary_histogram()
                        .map(|histogram| histogram.sample_count)
                        .unwrap_or(0),
                ),
                key_value(
                    "primary_bins",
                    "Primary Bins",
                    state
                        .primary_histogram()
                        .map(|histogram| histogram.bins.len())
                        .unwrap_or(0),
                ),
                key_value(
                    "real_mean",
                    "Real Mean",
                    format_estimate_with_error(state.real_mean(), state.real_stderr()),
                ),
                key_value(
                    "imag_mean",
                    "Imag Mean",
                    format_estimate_with_error(state.imag_mean(), state.imag_stderr()),
                ),
                key_value(
                    "abs_mean",
                    "Abs Mean",
                    format_estimate_with_error(state.abs_mean(), state.abs_stderr()),
                ),
                key_value(
                    "signal_to_noise",
                    "Mean(|x|)^2 / abs_err^2",
                    state.signal_to_noise(),
                ),
                key_value("rsd", "RSD", state.rsd()),
            ],
        ),
        ObservableState::FullScalar(state) => key_value_panel(
            "estimate_summary",
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
            "estimate_summary",
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

fn format_estimate_with_error(mean: f64, error: f64) -> String {
    if !mean.is_finite() || !error.is_finite() || error < 0.0 {
        return format!("{mean} +- {error}");
    }
    let scale_source = mean.abs().max(error.abs());
    if scale_source == 0.0 {
        return "(0 +- 0) x 10^0".to_string();
    }

    let exponent = scale_source.log10().floor() as i32;
    let scale = 10_f64.powi(exponent);
    let scaled_mean = mean / scale;
    let scaled_error = error / scale;
    let decimals = decimals_for_two_sig_figs(scaled_error);

    format!(
        "({mean} +- {error}) x 10^{exponent}",
        mean = format!("{scaled_mean:.decimals$}"),
        error = format!("{scaled_error:.decimals$}"),
    )
}

fn decimals_for_two_sig_figs(error: f64) -> usize {
    if error == 0.0 {
        return 0;
    }
    let exponent = error.abs().log10().floor() as i32;
    (1 - exponent).max(0) as usize
}

fn gammaloop_histogram_bundle_panel(observable: ObservableState) -> Option<PanelState> {
    let ObservableState::Gammaloop(state) = observable else {
        return None;
    };
    let payload = gammaloop_histogram_bundle_payload(&state);

    Some(table_panel_with_payload(
        "gammaloop_histogram_bundle",
        vec![
            "Name".to_string(),
            "Title".to_string(),
            "Phase".to_string(),
            "Transform".to_string(),
            "Samples".to_string(),
            "Bins".to_string(),
            "Range".to_string(),
            "In Range".to_string(),
            "Underflow".to_string(),
            "Overflow".to_string(),
            "NaN".to_string(),
            "Mitigated Pairs".to_string(),
            "Misbinning".to_string(),
            "Log X".to_string(),
            "Log Y".to_string(),
        ],
        state
            .bundle
            .histograms
            .iter()
            .map(|(name, histogram)| {
                vec![
                    JsonValue::String(name.clone()),
                    JsonValue::String(histogram.title.clone()),
                    JsonValue::String(match histogram.phase {
                        ObservablePhase::Real => "real".to_string(),
                        ObservablePhase::Imag => "imag".to_string(),
                    }),
                    JsonValue::String(match histogram.value_transform {
                        ObservableValueTransform::Identity => "identity".to_string(),
                        ObservableValueTransform::Log10 => "log10".to_string(),
                    }),
                    JsonValue::from(histogram.sample_count as i64),
                    JsonValue::from(histogram.bins.len() as i64),
                    JsonValue::String(format!("[{}, {}]", histogram.x_min, histogram.x_max)),
                    JsonValue::from(histogram.statistics.in_range_entry_count as i64),
                    JsonValue::from(histogram.underflow_bin.entry_count as i64),
                    JsonValue::from(histogram.overflow_bin.entry_count as i64),
                    JsonValue::from(histogram.statistics.nan_value_count as i64),
                    JsonValue::from(histogram.statistics.mitigated_pair_count as i64),
                    JsonValue::from(histogram.supports_misbinning_mitigation),
                    JsonValue::from(histogram.log_x_axis),
                    JsonValue::from(histogram.log_y_axis),
                ]
            })
            .collect(),
        Some(serde_json::to_value(payload).unwrap_or(JsonValue::Null)),
    ))
}

#[derive(Debug, Serialize)]
struct GammaloopHistogramBundlePayload {
    primary_histogram_name: Option<String>,
    histograms: BTreeMap<String, GammaloopHistogramSelectionEntry>,
}

#[derive(Debug, Serialize)]
struct GammaloopHistogramSelectionEntry {
    title: String,
    type_description: String,
    phase: String,
    value_transform: String,
    sample_count: usize,
    x_min: f64,
    x_max: f64,
    log_x_axis: bool,
    log_y_axis: bool,
    bins: Vec<HistogramBin>,
}

fn gammaloop_histogram_bundle_payload(
    state: &GammaLoopObservableState,
) -> GammaloopHistogramBundlePayload {
    GammaloopHistogramBundlePayload {
        primary_histogram_name: state.primary_histogram_name().map(str::to_string),
        histograms: state
            .bundle
            .histograms
            .iter()
            .map(|(name, histogram)| {
                (
                    name.clone(),
                    GammaloopHistogramSelectionEntry {
                        title: histogram.title.clone(),
                        type_description: histogram.type_description.clone(),
                        phase: match histogram.phase {
                            ObservablePhase::Real => "real".to_string(),
                            ObservablePhase::Imag => "imag".to_string(),
                        },
                        value_transform: match histogram.value_transform {
                            ObservableValueTransform::Identity => "identity".to_string(),
                            ObservableValueTransform::Log10 => "log10".to_string(),
                        },
                        sample_count: histogram.sample_count,
                        x_min: histogram.x_min,
                        x_max: histogram.x_max,
                        log_x_axis: histogram.log_x_axis,
                        log_y_axis: histogram.log_y_axis,
                        bins: histogram
                            .bins
                            .iter()
                            .map(|bin| HistogramBin {
                                start: bin.x_min.unwrap_or(histogram.x_min),
                                stop: bin.x_max.unwrap_or(histogram.x_max),
                                value: bin.average(histogram.sample_count),
                                error: bin.error(histogram.sample_count),
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::format_estimate_with_error;

    #[test]
    fn formats_estimate_with_matching_error_precision() {
        assert_eq!(
            format_estimate_with_error(1234.56, 12.34),
            "(1.235 +- 0.012) x 10^3"
        );
    }

    #[test]
    fn formats_zero_estimate_without_crashing() {
        assert_eq!(format_estimate_with_error(0.0, 0.0), "(0 +- 0) x 10^0");
    }

    #[test]
    fn preserves_two_significant_figures_for_integer_error() {
        assert_eq!(
            format_estimate_with_error(-9876.0, 100.0),
            "(-9.88 +- 0.10) x 10^3"
        );
    }
}
