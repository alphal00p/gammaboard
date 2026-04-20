use super::{
    TaskPanelContext, TaskPanelCurrentSourcePolicy, TaskPanelHistoryContext, TaskPanelProjector,
    panel_projector, panel_projector_with_source,
};
use crate::core::{
    AccumulatorConfig, EngineError, RunTaskSpec, SampleErrorProjection, SampleStopCondition,
};
use crate::evaluation::{Accumulator, AccumulatorState, SemanticAccumulatorKind};
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
    effective_accumulator_config: Option<AccumulatorConfig>,
) -> Vec<TaskPanelProjector> {
    let accumulator_config = task_accumulator_config(task_spec).or(effective_accumulator_config);
    let mut projectors = vec![
        sample_progress_projector(),
        estimate_summary_projector(accumulator_config.as_ref()),
        real_estimate_history_projector(accumulator_config.as_ref()),
    ];
    if matches!(
        accumulator_config,
        Some(AccumulatorConfig::Complex | AccumulatorConfig::Gammaloop)
    ) {
        projectors.push(imag_estimate_history_projector(accumulator_config.as_ref()));
    }
    projectors.push(rsd_history_projector(accumulator_config.as_ref()));
    if matches!(
        accumulator_config,
        Some(AccumulatorConfig::Gammaloop) | None
    ) {
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
    accumulator_config: Option<&AccumulatorConfig>,
) -> TaskPanelProjector {
    persisted_first_history_projector(
        "real_estimate_history",
        estimate_label(accumulator_config),
        accumulator_config.cloned(),
        |accumulator| Some(real_estimate_history_panel(accumulator)),
    )
}

fn imag_estimate_history_projector(
    accumulator_config: Option<&AccumulatorConfig>,
) -> TaskPanelProjector {
    persisted_first_history_projector(
        "imag_estimate_history",
        "Imaginary Mean",
        accumulator_config.cloned(),
        imag_estimate_history_panel,
    )
}

fn rsd_history_projector(accumulator_config: Option<&AccumulatorConfig>) -> TaskPanelProjector {
    persisted_first_history_projector(
        "abs_signal_to_noise_history",
        "RSD",
        accumulator_config.cloned(),
        |accumulator| Some(rsd_history_panel(accumulator)),
    )
}

fn persisted_first_history_projector<F>(
    panel_id: &'static str,
    label: &'static str,
    accumulator_config: Option<AccumulatorConfig>,
    map_panel: F,
) -> TaskPanelProjector
where
    F: Fn(AccumulatorState) -> Option<PanelState> + Copy + Send + Sync + 'static,
{
    let current_config = accumulator_config.clone();
    let history_config = accumulator_config;
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
        move |ctx| Ok(sample_accumulator(ctx, current_config.as_ref())?.and_then(map_panel)),
        move |ctx| Ok(decode_history_observable(ctx, history_config.as_ref())?.and_then(map_panel)),
    )
}

fn estimate_summary_projector(
    accumulator_config: Option<&AccumulatorConfig>,
) -> TaskPanelProjector {
    let accumulator_config = accumulator_config.cloned();
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
            Ok(
                sample_accumulator(ctx, accumulator_config.as_ref())?.map(|accumulator| {
                    estimate_summary_panel(
                        accumulator,
                        ctx.task.task.sample_stop_condition(),
                        ctx.task.nr_completed_samples,
                        ctx.completed_samples_per_second,
                    )
                }),
            )
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
            Ok(
                sample_accumulator(ctx, Some(&AccumulatorConfig::Gammaloop))?
                    .and_then(gammaloop_histogram_bundle_panel),
            )
        },
        |ctx| {
            Ok(
                decode_history_observable(ctx, Some(&AccumulatorConfig::Gammaloop))?
                    .and_then(gammaloop_histogram_bundle_panel),
            )
        },
    )
}

fn sample_progress_value(ctx: &TaskPanelContext<'_>) -> f64 {
    ctx.task.nr_completed_samples.max(0) as f64
}

fn sample_accumulator(
    ctx: &TaskPanelContext<'_>,
    accumulator_config: Option<&AccumulatorConfig>,
) -> Result<Option<AccumulatorState>, EngineError> {
    if let Some(accumulator) = ctx.source.accumulator() {
        let requested_config = accumulator_config.cloned();
        if requested_config
            .as_ref()
            .map(|config| accumulator_matches_requested_config(accumulator, config))
            .unwrap_or(true)
        {
            return Ok(Some(accumulator.clone()));
        }
        if ctx.source.persisted().is_none() {
            return Err(EngineError::build(format!(
                "accumulator type mismatch: expected {}, got {} and no persisted snapshot was available for fallback decoding",
                config_label(&requested_config.expect("checked is_some above")),
                accumulator.kind_str()
            )));
        }
    }
    match ctx.source.persisted() {
        Some(persisted) => {
            decode_aggregate_persisted_accumulator_with_fallback(accumulator_config, persisted)
        }
        None => Ok(None),
    }
}

fn accumulator_matches_requested_config(
    accumulator: &AccumulatorState,
    requested: &AccumulatorConfig,
) -> bool {
    match requested {
        AccumulatorConfig::Empty => matches!(accumulator, AccumulatorState::Empty(_)),
        AccumulatorConfig::Scalar => matches!(
            accumulator,
            AccumulatorState::Scalar(_) | AccumulatorState::FullScalar(_)
        ),
        AccumulatorConfig::Complex => matches!(
            accumulator,
            AccumulatorState::Complex(_) | AccumulatorState::FullComplex(_)
        ),
        AccumulatorConfig::Gammaloop => matches!(accumulator, AccumulatorState::Gammaloop(_)),
        AccumulatorConfig::FullScalar => matches!(accumulator, AccumulatorState::FullScalar(_)),
        AccumulatorConfig::FullComplex => matches!(accumulator, AccumulatorState::FullComplex(_)),
    }
}

fn decode_history_observable(
    ctx: &TaskPanelHistoryContext<'_>,
    accumulator_config: Option<&AccumulatorConfig>,
) -> Result<Option<AccumulatorState>, EngineError> {
    decode_aggregate_persisted_accumulator_with_fallback(
        accumulator_config,
        &ctx.snapshot.persisted_output,
    )
}

fn decode_aggregate_persisted_accumulator(
    config: &AccumulatorConfig,
    persisted: &JsonValue,
) -> Result<AccumulatorState, EngineError> {
    match config {
        AccumulatorConfig::Empty => Err(EngineError::build(
            "sample task expected aggregate accumulator, got empty".to_string(),
        )),
        AccumulatorConfig::Scalar => AccumulatorState::from_aggregate_persistent_json(
            SemanticAccumulatorKind::Scalar,
            persisted,
        ),
        AccumulatorConfig::Complex => AccumulatorState::from_aggregate_persistent_json(
            SemanticAccumulatorKind::Complex,
            persisted,
        ),
        AccumulatorConfig::Gammaloop => AccumulatorState::from_gammaloop_persistent_json(persisted),
        AccumulatorConfig::FullScalar | AccumulatorConfig::FullComplex => {
            Err(EngineError::build(format!(
                "sample task expected aggregate accumulator, got {}",
                config_label(config)
            )))
        }
    }
}

fn decode_aggregate_persisted_accumulator_with_fallback(
    accumulator_config: Option<&AccumulatorConfig>,
    persisted: &JsonValue,
) -> Result<Option<AccumulatorState>, EngineError> {
    if let Some(config) = accumulator_config {
        return decode_aggregate_persisted_accumulator(config, persisted).map(Some);
    }
    if let Ok(accumulator) =
        decode_aggregate_persisted_accumulator(&AccumulatorConfig::Scalar, persisted)
    {
        return Ok(Some(accumulator));
    }
    if let Ok(accumulator) =
        decode_aggregate_persisted_accumulator(&AccumulatorConfig::Complex, persisted)
    {
        return Ok(Some(accumulator));
    }
    if let Ok(accumulator) =
        decode_aggregate_persisted_accumulator(&AccumulatorConfig::Gammaloop, persisted)
    {
        return Ok(Some(accumulator));
    }
    Ok(None)
}

fn estimate_label(accumulator_config: Option<&AccumulatorConfig>) -> &'static str {
    match accumulator_config {
        Some(AccumulatorConfig::Empty) => "Estimate",
        Some(AccumulatorConfig::Scalar) => "Mean",
        Some(AccumulatorConfig::Complex) => "Real Mean",
        Some(AccumulatorConfig::Gammaloop) => "Real Mean",
        None => "Estimate",
        Some(AccumulatorConfig::FullScalar) | Some(AccumulatorConfig::FullComplex) => "Estimate",
    }
}

fn task_accumulator_config(task: &RunTaskSpec) -> Option<AccumulatorConfig> {
    task.new_accumulator_config().ok().flatten()
}

fn config_label(config: &AccumulatorConfig) -> &'static str {
    match config {
        AccumulatorConfig::Empty => "empty",
        AccumulatorConfig::Scalar => "scalar",
        AccumulatorConfig::Complex => "complex",
        AccumulatorConfig::Gammaloop => "gammaloop",
        AccumulatorConfig::FullScalar => "full_scalar",
        AccumulatorConfig::FullComplex => "full_complex",
    }
}

fn real_estimate_history_panel(accumulator: AccumulatorState) -> PanelState {
    let smooth = Some(true);
    match accumulator {
        AccumulatorState::Scalar(state) => scalar_timeseries_panel_with_smoothing(
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
        AccumulatorState::Complex(state) => scalar_timeseries_panel_with_smoothing(
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
        AccumulatorState::Gammaloop(state) => scalar_timeseries_panel_with_smoothing(
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

fn imag_estimate_history_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let smooth = Some(true);
    match accumulator {
        AccumulatorState::Complex(state) => Some(scalar_timeseries_panel_with_smoothing(
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
        AccumulatorState::Gammaloop(state) => Some(scalar_timeseries_panel_with_smoothing(
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

fn rsd_history_panel(accumulator: AccumulatorState) -> PanelState {
    let rsd = match &accumulator {
        AccumulatorState::Scalar(state) => state.rsd(),
        AccumulatorState::Complex(state) => state.rsd(),
        AccumulatorState::Gammaloop(state) => state.rsd(),
        _ => 0.0,
    };
    scalar_timeseries_panel_with_smoothing(
        "abs_signal_to_noise_history",
        vec![PlotPoint {
            x: accumulator.sample_count() as f64,
            y: rsd,
            x_sampler_uptime_ms: None,
            x_completed_samples_total: None,
            y_min: None,
            y_max: None,
        }],
        Some(true),
    )
}

fn estimate_summary_panel(
    accumulator: AccumulatorState,
    stop_condition: Option<&SampleStopCondition>,
    completed_samples: i64,
    completed_samples_per_second: Option<f64>,
) -> PanelState {
    let mut entries = base_estimate_summary_entries(&accumulator);
    append_stop_condition_entries(
        &mut entries,
        stop_condition,
        &accumulator,
        completed_samples,
        completed_samples_per_second,
    );
    key_value_panel("estimate_summary", entries)
}

fn base_estimate_summary_entries(
    accumulator: &AccumulatorState,
) -> Vec<crate::server::panels::KeyValueEntry> {
    match accumulator {
        AccumulatorState::Empty(_) => vec![key_value("count", "Count", 0)],
        AccumulatorState::Scalar(state) => vec![
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
        AccumulatorState::Complex(state) => vec![
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
        AccumulatorState::Gammaloop(state) => vec![
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
        AccumulatorState::FullScalar(state) => vec![
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
        AccumulatorState::FullComplex(state) => vec![
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
    }
}

fn append_stop_condition_entries(
    entries: &mut Vec<crate::server::panels::KeyValueEntry>,
    stop_condition: Option<&SampleStopCondition>,
    accumulator: &AccumulatorState,
    completed_samples: i64,
    completed_samples_per_second: Option<f64>,
) {
    let Some(stop_condition) = stop_condition else {
        return;
    };
    let projection = stop_condition
        .projection
        .unwrap_or_else(|| match accumulator {
            AccumulatorState::Complex(_) | AccumulatorState::Gammaloop(_) => {
                SampleErrorProjection::Abs
            }
            _ => SampleErrorProjection::Real,
        });
    entries.push(key_value(
        "stop_projection",
        "Stop Projection",
        format_projection(projection),
    ));
    if let Some(target) = stop_condition.max_samples {
        entries.push(key_value("stop_max_samples", "Stop Max Samples", target));
    }

    let projected = projected_estimate(accumulator, projection);
    if let Some(projected) = projected {
        if let Some(target) = stop_condition.absolute_error {
            entries.push(key_value(
                "stop_absolute_error_current",
                "Absolute Error",
                projected.error,
            ));
            entries.push(key_value(
                "stop_absolute_error_target",
                "Absolute Error Target",
                target,
            ));
        }
        if let Some(target) = stop_condition.relative_error {
            let current = relative_error(projected.value, projected.error);
            entries.push(key_value(
                "stop_relative_error_current",
                "Relative Error",
                current,
            ));
            entries.push(key_value(
                "stop_relative_error_target",
                "Relative Error Target",
                target,
            ));
        }
    }

    let eta_seconds = estimate_eta_seconds(
        stop_condition,
        projected,
        completed_samples,
        completed_samples_per_second,
    );
    entries.push(key_value(
        "eta",
        "ETA",
        eta_seconds
            .map(format_eta_duration)
            .unwrap_or_else(|| "n/a".to_string()),
    ));
}

#[derive(Debug, Clone, Copy)]
struct ProjectedEstimate {
    value: f64,
    error: f64,
}

fn projected_estimate(
    accumulator: &AccumulatorState,
    projection: SampleErrorProjection,
) -> Option<ProjectedEstimate> {
    match accumulator {
        AccumulatorState::Scalar(state) => match projection {
            SampleErrorProjection::Real => Some(ProjectedEstimate {
                value: state.mean(),
                error: state.stderr(),
            }),
            SampleErrorProjection::Imag | SampleErrorProjection::Abs => None,
        },
        AccumulatorState::Complex(state) => match projection {
            SampleErrorProjection::Real => Some(ProjectedEstimate {
                value: state.real_mean(),
                error: state.real_stderr(),
            }),
            SampleErrorProjection::Imag => Some(ProjectedEstimate {
                value: state.imag_mean(),
                error: state.imag_stderr(),
            }),
            SampleErrorProjection::Abs => Some(ProjectedEstimate {
                value: state.abs_mean(),
                error: state.abs_stderr(),
            }),
        },
        AccumulatorState::Gammaloop(state) => match projection {
            SampleErrorProjection::Real => Some(ProjectedEstimate {
                value: state.real_mean(),
                error: state.real_stderr(),
            }),
            SampleErrorProjection::Imag => Some(ProjectedEstimate {
                value: state.imag_mean(),
                error: state.imag_stderr(),
            }),
            SampleErrorProjection::Abs => Some(ProjectedEstimate {
                value: state.abs_mean(),
                error: state.abs_stderr(),
            }),
        },
        AccumulatorState::Empty(_)
        | AccumulatorState::FullScalar(_)
        | AccumulatorState::FullComplex(_) => None,
    }
}

fn relative_error(value: f64, abs_error: f64) -> f64 {
    let denominator = value.abs();
    if denominator == 0.0 {
        if abs_error == 0.0 { 0.0 } else { f64::INFINITY }
    } else {
        abs_error.abs() / denominator
    }
}

fn estimate_eta_seconds(
    stop_condition: &SampleStopCondition,
    projected: Option<ProjectedEstimate>,
    completed_samples: i64,
    completed_samples_per_second: Option<f64>,
) -> Option<f64> {
    let rate = completed_samples_per_second.filter(|value| value.is_finite() && *value > 0.0)?;
    let completed_samples = completed_samples.max(0) as f64;
    let mut etas = Vec::new();
    if let Some(max_samples) = stop_condition.max_samples {
        let remaining = (max_samples as f64 - completed_samples).max(0.0);
        etas.push(remaining / rate);
    }
    if let (Some(target), Some(projected)) = (stop_condition.absolute_error, projected) {
        if projected.error <= target {
            etas.push(0.0);
        } else if completed_samples > 0.0 && projected.error.is_finite() && target.is_finite() {
            let required_total = completed_samples * (projected.error / target).powi(2);
            let remaining = (required_total - completed_samples).max(0.0);
            etas.push(remaining / rate);
        }
    }
    if let (Some(target), Some(projected)) = (stop_condition.relative_error, projected) {
        let current_relative = relative_error(projected.value, projected.error);
        if current_relative <= target {
            etas.push(0.0);
        } else if completed_samples > 0.0 && current_relative.is_finite() && target.is_finite() {
            let required_total = completed_samples * (current_relative / target).powi(2);
            let remaining = (required_total - completed_samples).max(0.0);
            etas.push(remaining / rate);
        }
    }
    etas.into_iter().reduce(f64::min)
}

fn format_projection(value: SampleErrorProjection) -> &'static str {
    match value {
        SampleErrorProjection::Real => "real",
        SampleErrorProjection::Imag => "imag",
        SampleErrorProjection::Abs => "abs",
    }
}

fn format_eta_duration(seconds: f64) -> String {
    if !seconds.is_finite() {
        return "n/a".to_string();
    }
    if seconds < 60.0 {
        return format!("{seconds:.2} s");
    }
    if seconds < 3600.0 {
        return format!("{:.2} min", seconds / 60.0);
    }
    if seconds < 365.0 * 24.0 * 3600.0 {
        return format!("{:.2} h", seconds / 3600.0);
    }
    let years = seconds / (365.0 * 24.0 * 3600.0);
    if years >= 1_000_000.0 {
        return format!("{years:.3e} y");
    }
    format!("{years:.2} y")
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

fn gammaloop_histogram_bundle_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let AccumulatorState::Gammaloop(state) = accumulator else {
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
