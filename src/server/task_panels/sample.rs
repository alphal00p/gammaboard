use super::{
    TaskPanelContext, TaskPanelCurrentSourcePolicy, TaskPanelHistoryContext, TaskPanelProjector,
    panel_projector, panel_projector_with_source,
};
use crate::core::{EngineError, ObservableConfig, RunTaskSpec};
use crate::evaluation::{Observable, ObservableState, SemanticObservableKind};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelState, PanelWidth, PlotPoint, key_value, key_value_panel,
    panel_spec, progress_panel, scalar_timeseries_panel_with_smoothing, table_panel_with_payload,
    with_panel_width,
};
use gammalooprs::observables::{ObservablePhase, ObservableValueTransform};
use serde::Serialize;
use serde_json::Value as JsonValue;

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
    if matches!(observable_config, Some(ObservableConfig::Gammaloop) | None) {
        projectors.push(gammaloop_histogram_bundle_projector());
    }
    projectors
}

fn sample_progress_projector() -> TaskPanelProjector {
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "sample_progress",
                "Sample Progress",
                PanelKind::Progress,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        |ctx| {
            let current = sample_progress_value(ctx);
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
    persisted_first_history_projector(
        "real_estimate_history",
        estimate_label(observable_config),
        observable_config.cloned(),
        |observable| Some(real_estimate_history_panel(observable)),
    )
}

fn imag_estimate_history_projector(
    observable_config: Option<&ObservableConfig>,
) -> TaskPanelProjector {
    persisted_first_history_projector(
        "imag_estimate_history",
        "Imaginary Mean",
        observable_config.cloned(),
        imag_estimate_history_panel,
    )
}

fn abs_signal_to_noise_history_projector(
    observable_config: Option<&ObservableConfig>,
) -> TaskPanelProjector {
    persisted_first_history_projector(
        "abs_signal_to_noise_history",
        "Mean(|x|)^2 / abs_err^2",
        observable_config.cloned(),
        |observable| Some(abs_signal_to_noise_panel(observable)),
    )
}

fn persisted_first_history_projector<F>(
    panel_id: &'static str,
    label: &'static str,
    observable_config: Option<ObservableConfig>,
    map_panel: F,
) -> TaskPanelProjector
where
    F: Fn(ObservableState) -> Option<PanelState> + Copy + Send + Sync + 'static,
{
    let current_config = observable_config.clone();
    let history_config = observable_config;
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                panel_id,
                label,
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| Ok(sample_observable(ctx, current_config.as_ref())?.and_then(map_panel)),
        move |ctx| Ok(decode_history_observable(ctx, history_config.as_ref())?.and_then(map_panel)),
    )
}

fn estimate_summary_projector(observable_config: Option<&ObservableConfig>) -> TaskPanelProjector {
    let observable_config = observable_config.cloned();
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "estimate_summary",
                "Estimate Summary",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
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

fn sample_progress_value(ctx: &TaskPanelContext<'_>) -> f64 {
    ctx.task.nr_completed_samples.max(0) as f64
}

fn sample_observable(
    ctx: &TaskPanelContext<'_>,
    observable_config: Option<&ObservableConfig>,
) -> Result<Option<ObservableState>, EngineError> {
    if let Some(observable) = ctx.source.observable() {
        let requested_config = observable_config.cloned();
        if requested_config
            .as_ref()
            .map(|config| observable_matches_requested_config(observable, config))
            .unwrap_or(true)
        {
            return Ok(Some(observable.clone()));
        }
        if ctx.source.persisted().is_none() {
            return Err(EngineError::build(format!(
                "observable type mismatch: expected {}, got {} and no persisted snapshot was available for fallback decoding",
                config_label(&requested_config.expect("checked is_some above")),
                observable.kind_str()
            )));
        }
    }
    match ctx.source.persisted() {
        Some(persisted) => {
            decode_aggregate_persisted_observable_with_fallback(observable_config, persisted)
        }
        None => Ok(None),
    }
}

fn observable_matches_requested_config(
    observable: &ObservableState,
    requested: &ObservableConfig,
) -> bool {
    match requested {
        ObservableConfig::Empty => matches!(observable, ObservableState::Empty(_)),
        ObservableConfig::Scalar => matches!(
            observable,
            ObservableState::Scalar(_) | ObservableState::FullScalar(_)
        ),
        ObservableConfig::Complex => matches!(
            observable,
            ObservableState::Complex(_) | ObservableState::FullComplex(_)
        ),
        ObservableConfig::Gammaloop => matches!(observable, ObservableState::Gammaloop(_)),
        ObservableConfig::FullScalar => matches!(observable, ObservableState::FullScalar(_)),
        ObservableConfig::FullComplex => matches!(observable, ObservableState::FullComplex(_)),
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
        ObservableConfig::Empty => Err(EngineError::build(
            "sample task expected aggregate observable, got empty".to_string(),
        )),
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
        Some(ObservableConfig::Empty) => "Estimate",
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
        ObservableConfig::Empty => "empty",
        ObservableConfig::Scalar => "scalar",
        ObservableConfig::Complex => "complex",
        ObservableConfig::Gammaloop => "gammaloop",
        ObservableConfig::FullScalar => "full_scalar",
        ObservableConfig::FullComplex => "full_complex",
    }
}

fn real_estimate_history_panel(observable: ObservableState) -> PanelState {
    let smooth = Some(true);
    match observable {
        ObservableState::Scalar(state) => scalar_timeseries_panel_with_smoothing(
            "real_estimate_history",
            vec![PlotPoint {
                x: state.count as f64,
                y: state.mean(),
                x_sampler_uptime_ms: None,
                x_completed_samples_total: None,
                y_min: Some(state.mean() - state.stderr()),
                y_max: Some(state.mean() + state.stderr()),
            }],
            smooth,
        ),
        ObservableState::Complex(state) => scalar_timeseries_panel_with_smoothing(
            "real_estimate_history",
            vec![PlotPoint {
                x: state.count as f64,
                y: state.real_mean(),
                x_sampler_uptime_ms: None,
                x_completed_samples_total: None,
                y_min: Some(state.real_mean() - state.real_stderr()),
                y_max: Some(state.real_mean() + state.real_stderr()),
            }],
            smooth,
        ),
        ObservableState::Gammaloop(state) => scalar_timeseries_panel_with_smoothing(
            "real_estimate_history",
            vec![PlotPoint {
                x: state.sample_count() as f64,
                y: state.real_mean(),
                x_sampler_uptime_ms: None,
                x_completed_samples_total: None,
                y_min: Some(state.real_mean() - state.real_stderr()),
                y_max: Some(state.real_mean() + state.real_stderr()),
            }],
            smooth,
        ),
        _ => scalar_timeseries_panel_with_smoothing("real_estimate_history", Vec::new(), smooth),
    }
}

fn imag_estimate_history_panel(observable: ObservableState) -> Option<PanelState> {
    let smooth = Some(true);
    match observable {
        ObservableState::Complex(state) => Some(scalar_timeseries_panel_with_smoothing(
            "imag_estimate_history",
            vec![PlotPoint {
                x: state.count as f64,
                y: state.imag_mean(),
                x_sampler_uptime_ms: None,
                x_completed_samples_total: None,
                y_min: Some(state.imag_mean() - state.imag_stderr()),
                y_max: Some(state.imag_mean() + state.imag_stderr()),
            }],
            smooth,
        )),
        ObservableState::Gammaloop(state) => Some(scalar_timeseries_panel_with_smoothing(
            "imag_estimate_history",
            vec![PlotPoint {
                x: state.sample_count() as f64,
                y: state.imag_mean(),
                x_sampler_uptime_ms: None,
                x_completed_samples_total: None,
                y_min: Some(state.imag_mean() - state.imag_stderr()),
                y_max: Some(state.imag_mean() + state.imag_stderr()),
            }],
            smooth,
        )),
        _ => None,
    }
}

fn abs_signal_to_noise_panel(observable: ObservableState) -> PanelState {
    scalar_timeseries_panel_with_smoothing(
        "abs_signal_to_noise_history",
        vec![PlotPoint {
            x: observable.sample_count() as f64,
            y: observable.abs_signal_to_noise(),
            x_sampler_uptime_ms: None,
            x_completed_samples_total: None,
            y_min: None,
            y_max: None,
        }],
        Some(true),
    )
}

fn estimate_summary_panel(observable: ObservableState) -> PanelState {
    match observable {
        ObservableState::Empty(_) => {
            key_value_panel("estimate_summary", vec![key_value("count", "Count", 0)])
        }
        ObservableState::Scalar(state) => key_value_panel(
            "estimate_summary",
            vec![
                key_value("count", "Count", state.count),
                key_value("mean", "Mean", estimate_value(state.mean(), state.stderr())),
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
                    estimate_value(state.real_mean(), state.real_stderr()),
                ),
                key_value(
                    "imag_mean",
                    "Imag Mean",
                    estimate_value(state.imag_mean(), state.imag_stderr()),
                ),
                key_value(
                    "abs_mean",
                    "Abs Mean",
                    estimate_value(state.abs_mean(), state.abs_stderr()),
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
                key_value(
                    "real_mean",
                    "Real Mean",
                    estimate_value(state.real_mean(), state.real_stderr()),
                ),
                key_value(
                    "imag_mean",
                    "Imag Mean",
                    estimate_value(state.imag_mean(), state.imag_stderr()),
                ),
                key_value(
                    "abs_mean",
                    "Abs Mean",
                    estimate_value(state.abs_mean(), state.abs_stderr()),
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

#[derive(Debug, Serialize)]
struct EstimateValuePayload {
    kind: &'static str,
    value: f64,
    error: f64,
}

fn estimate_value(value: f64, error: f64) -> EstimateValuePayload {
    EstimateValuePayload {
        kind: "estimate",
        value,
        error,
    }
}

fn gammaloop_histogram_bundle_panel(observable: ObservableState) -> Option<PanelState> {
    let ObservableState::Gammaloop(state) = observable else {
        return None;
    };
    let payload = serde_json::to_value(&state.bundle).unwrap_or(JsonValue::Null);

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
                    JsonValue::String(match histogram.kind {
                        gammalooprs::observables::HistogramSnapshotKind::Continuous => {
                            match (histogram.x_min, histogram.x_max) {
                                (Some(x_min), Some(x_max)) => format!("[{}, {}]", x_min, x_max),
                                _ => "continuous".to_string(),
                            }
                        }
                        gammalooprs::observables::HistogramSnapshotKind::Discrete => histogram
                            .discrete_min_bin_id
                            .map(|min_bin_id| {
                                format!(
                                    "[{}, {}]",
                                    min_bin_id,
                                    min_bin_id + histogram.bins.len() as isize
                                )
                            })
                            .unwrap_or_else(|| "discrete".to_string()),
                    }),
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
        Some(payload),
    ))
}
