use super::{TaskPanelContext, TaskPanelHistoryContext, TaskPanelProjector, panel_projector};
use crate::core::{EngineError, RunSpec};
use crate::evaluation::{FullObservableProgress, ObservableState, SemanticObservableKind};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelState, PlotPoint, key_value, key_value_panel, panel_spec,
    progress_panel, scalar_timeseries_panel, single_point_band,
};
use serde_json::Value as JsonValue;

pub(super) fn projectors(run_spec: &RunSpec) -> Vec<TaskPanelProjector> {
    let observable_kind = run_spec.evaluator.observable_kind();
    let mut projectors = vec![
        sample_progress_projector(),
        real_estimate_history_projector(observable_kind),
    ];
    if matches!(observable_kind, SemanticObservableKind::Complex) {
        projectors.push(imag_estimate_history_projector());
    }
    projectors.push(abs_signal_to_noise_history_projector(observable_kind));
    projectors.push(estimate_summary_projector(observable_kind));
    projectors
}

fn sample_progress_projector() -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            "sample_progress",
            "Sample Progress",
            PanelKind::Progress,
            PanelHistoryMode::None,
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

fn real_estimate_history_projector(observable_kind: SemanticObservableKind) -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            "real_estimate_history",
            estimate_label(observable_kind),
            PanelKind::ScalarTimeseries,
            PanelHistoryMode::Append,
        ),
        move |ctx| Ok(sample_observable(ctx, observable_kind)?.map(real_estimate_history_panel)),
        move |ctx| {
            Ok(decode_history_observable(ctx, observable_kind)?.map(real_estimate_history_panel))
        },
    )
}

fn imag_estimate_history_projector() -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            "imag_estimate_history",
            "Imaginary Mean",
            PanelKind::ScalarTimeseries,
            PanelHistoryMode::Append,
        ),
        |ctx| {
            Ok(sample_observable(ctx, SemanticObservableKind::Complex)?
                .and_then(imag_estimate_history_panel))
        },
        |ctx| {
            Ok(
                decode_history_observable(ctx, SemanticObservableKind::Complex)?
                    .and_then(imag_estimate_history_panel),
            )
        },
    )
}

fn abs_signal_to_noise_history_projector(
    observable_kind: SemanticObservableKind,
) -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            "abs_signal_to_noise_history",
            "Mean(|x|)^2 / abs_err^2",
            PanelKind::ScalarTimeseries,
            PanelHistoryMode::Append,
        ),
        move |ctx| Ok(sample_observable(ctx, observable_kind)?.map(abs_signal_to_noise_panel)),
        move |ctx| {
            Ok(decode_history_observable(ctx, observable_kind)?.map(abs_signal_to_noise_panel))
        },
    )
}

fn estimate_summary_projector(observable_kind: SemanticObservableKind) -> TaskPanelProjector {
    panel_projector(
        panel_spec(
            "estimate_summary",
            "Estimate Summary",
            PanelKind::KeyValue,
            PanelHistoryMode::None,
        ),
        move |ctx| Ok(sample_observable(ctx, observable_kind)?.map(estimate_summary_panel)),
        |_ctx| Ok(None),
    )
}

fn sample_progress_value(ctx: &TaskPanelContext<'_>) -> Result<f64, EngineError> {
    if let Some(persisted) = ctx.source.persisted() {
        if let Ok(observable) = decode_aggregate_persisted_observable(
            ctx.run_spec.evaluator.observable_kind(),
            persisted,
        ) {
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
    kind: SemanticObservableKind,
) -> Result<Option<ObservableState>, EngineError> {
    if let Some(observable) = ctx.source.observable() {
        return Ok(Some(observable.clone()));
    }
    match ctx.source.persisted() {
        Some(persisted) => decode_aggregate_persisted_observable(kind, persisted).map(Some),
        None => Ok(None),
    }
}

fn decode_history_observable(
    ctx: &TaskPanelHistoryContext<'_>,
    kind: SemanticObservableKind,
) -> Result<Option<ObservableState>, EngineError> {
    decode_aggregate_persisted_observable(kind, &ctx.snapshot.persisted_output).map(Some)
}

fn decode_aggregate_persisted_observable(
    kind: SemanticObservableKind,
    persisted: &JsonValue,
) -> Result<ObservableState, EngineError> {
    ObservableState::from_aggregate_persistent_json(kind, persisted)
}

fn decode_full_progress(persisted: &JsonValue) -> Result<FullObservableProgress, EngineError> {
    serde_json::from_value(persisted.clone())
        .map_err(|err| EngineError::build(format!("invalid full observable progress: {err}")))
}

fn estimate_label(observable_kind: SemanticObservableKind) -> &'static str {
    match observable_kind {
        SemanticObservableKind::Scalar => "Mean",
        SemanticObservableKind::Complex => "Real Mean",
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
