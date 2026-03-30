use super::{TaskPanelContext, TaskPanelHistoryContext, TaskPanelProjector, panel_projector};
use crate::core::{EngineError, ObservableConfig, RunTaskSpec};
use crate::evaluation::{
    FullObservableProgress, Observable, ObservableState, SemanticObservableKind,
};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelState, PanelWidth, PlotPoint, key_value, key_value_panel,
    panel_spec, progress_panel, scalar_timeseries_panel, single_point_band, with_panel_width,
};
use serde_json::Value as JsonValue;

pub(super) fn projectors(task_spec: &RunTaskSpec) -> Vec<TaskPanelProjector> {
    let observable_config = task_observable_config(task_spec);
    let mut projectors = vec![
        sample_progress_projector(),
        estimate_summary_projector(observable_config.as_ref()),
        real_estimate_history_projector(observable_config.as_ref()),
    ];
    if matches!(
        observable_config,
        Some(ObservableConfig::Complex | ObservableConfig::Gammaloop)
    ) {
        projectors.push(imag_estimate_history_projector());
    }
    projectors.push(abs_signal_to_noise_history_projector(
        observable_config.as_ref(),
    ));
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

fn imag_estimate_history_projector() -> TaskPanelProjector {
    let observable_config = Some(ObservableConfig::Complex);
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
                key_value("mean", "Mean", state.mean()),
                key_value("error", "Error", state.stderr()),
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
                key_value("real_mean", "Real Mean", state.real_mean()),
                key_value("imag_mean", "Imag Mean", state.imag_mean()),
                key_value("real_error", "Real Error", state.real_stderr()),
                key_value("imag_error", "Imag Error", state.imag_stderr()),
                key_value("abs_mean", "Abs Mean", state.abs_mean()),
                key_value("abs_error", "Abs Error", state.abs_stderr()),
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
                key_value("real_mean", "Real Mean", state.real_mean()),
                key_value("imag_mean", "Imag Mean", state.imag_mean()),
                key_value("real_error", "Real Error", state.real_stderr()),
                key_value("imag_error", "Imag Error", state.imag_stderr()),
                key_value("abs_mean", "Abs Mean", state.abs_mean()),
                key_value("abs_error", "Abs Error", state.abs_stderr()),
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
