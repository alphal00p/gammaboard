use crate::core::{EngineError, RunSpec, RunTask};
use crate::evaluation::{FullObservableProgress, ObservableState, SemanticObservableKind};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelSpec, PanelState, PlotPoint, key_value, key_value_panel,
    panel_spec, progress_panel, scalar_timeseries_panel, single_point_band,
};
use serde_json::Value as JsonValue;

pub(super) fn panel_specs(run_spec: &RunSpec) -> Vec<PanelSpec> {
    let mut panels = vec![panel_spec(
        "sample_progress",
        "Sample Progress",
        PanelKind::Progress,
        PanelHistoryMode::None,
    )];
    panels.push(panel_spec(
        "real_estimate_history",
        estimate_label(run_spec),
        PanelKind::ScalarTimeseries,
        PanelHistoryMode::Append,
    ));
    if matches!(
        run_spec.evaluator.observable_kind(),
        SemanticObservableKind::Complex
    ) {
        panels.push(panel_spec(
            "imag_estimate_history",
            "Imaginary Mean",
            PanelKind::ScalarTimeseries,
            PanelHistoryMode::Append,
        ));
    }
    panels.push(panel_spec(
        "abs_signal_to_noise_history",
        "Mean(|x|)^2 / abs_err^2",
        PanelKind::ScalarTimeseries,
        PanelHistoryMode::Append,
    ));
    panels.push(panel_spec(
        "estimate_summary",
        "Estimate Summary",
        PanelKind::KeyValue,
        PanelHistoryMode::None,
    ));
    panels
}

pub(super) fn build_current_panels(
    task: &RunTask,
    observable: Option<&ObservableState>,
    run_spec: &RunSpec,
) -> Result<Vec<PanelState>, EngineError> {
    Ok(build_panels(
        task.nr_completed_samples as f64,
        task.task.nr_expected_samples().map(|value| value as f64),
        observable,
        run_spec,
    ))
}

pub(super) fn build_panels_from_persisted(
    persisted: &JsonValue,
    progress_total: Option<f64>,
    run_spec: &RunSpec,
) -> Result<Vec<PanelState>, EngineError> {
    if let Ok(observable) =
        decode_aggregate_persisted_observable(run_spec.evaluator.observable_kind(), persisted)
    {
        return Ok(build_panels(
            observable.sample_count() as f64,
            progress_total,
            Some(&observable),
            run_spec,
        ));
    }

    if let Ok(progress) = decode_full_progress(persisted) {
        return Ok(build_panels(
            progress.processed as f64,
            progress_total,
            None,
            run_spec,
        ));
    }

    decode_aggregate_persisted_observable(run_spec.evaluator.observable_kind(), persisted).map(
        |observable| {
            build_panels(
                observable.sample_count() as f64,
                progress_total,
                Some(&observable),
                run_spec,
            )
        },
    )
}

fn build_panels(
    progress_current: f64,
    progress_total: Option<f64>,
    observable: Option<&ObservableState>,
    run_spec: &RunSpec,
) -> Vec<PanelState> {
    let mut panels = vec![progress_panel(
        "sample_progress",
        progress_current,
        progress_total,
        Some("samples"),
    )];
    if let Some(observable) = observable {
        panels.extend(build_estimate_panels_from_observable(observable, run_spec));
        panels.push(build_abs_signal_to_noise_panel_from_observable(observable));
        panels.push(build_summary_panel_from_observable(
            "estimate_summary",
            observable,
        ));
    }
    panels
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

fn estimate_label(run_spec: &RunSpec) -> &'static str {
    match run_spec.evaluator.observable_kind() {
        SemanticObservableKind::Scalar => "Mean",
        SemanticObservableKind::Complex => "Real Mean",
    }
}

fn build_estimate_panels_from_observable(
    observable: &ObservableState,
    run_spec: &RunSpec,
) -> Vec<PanelState> {
    match run_spec.evaluator.observable_kind() {
        SemanticObservableKind::Scalar => vec![single_point_band(
            "real_estimate_history",
            observable.sample_count() as f64,
            scalar_estimate(observable),
            Some(scalar_estimate(observable) - scalar_error(observable)),
            Some(scalar_estimate(observable) + scalar_error(observable)),
        )],
        SemanticObservableKind::Complex => complex_estimate_panels(observable),
    }
}

fn build_abs_signal_to_noise_panel_from_observable(observable: &ObservableState) -> PanelState {
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

fn build_summary_panel_from_observable(panel_id: &str, observable: &ObservableState) -> PanelState {
    match observable {
        ObservableState::Scalar(state) => key_value_panel(
            panel_id,
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
            panel_id,
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
            panel_id,
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
            panel_id,
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

fn scalar_estimate(observable: &ObservableState) -> f64 {
    match observable {
        ObservableState::Scalar(state) => state.mean(),
        _ => 0.0,
    }
}

fn scalar_error(observable: &ObservableState) -> f64 {
    match observable {
        ObservableState::Scalar(state) => state.stderr(),
        _ => 0.0,
    }
}

fn complex_estimate_panels(observable: &ObservableState) -> Vec<PanelState> {
    match observable {
        ObservableState::Complex(state) => vec![
            single_point_band(
                "real_estimate_history",
                state.count as f64,
                state.real_mean(),
                Some(state.real_mean() - state.real_stderr()),
                Some(state.real_mean() + state.real_stderr()),
            ),
            single_point_band(
                "imag_estimate_history",
                state.count as f64,
                state.imag_mean(),
                Some(state.imag_mean() - state.imag_stderr()),
                Some(state.imag_mean() + state.imag_stderr()),
            ),
        ],
        _ => vec![],
    }
}
