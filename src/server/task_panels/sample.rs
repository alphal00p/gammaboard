use super::{
    TaskPanelContext, TaskPanelCurrentSourcePolicy, TaskPanelHistoryContext, TaskPanelProjector,
    panel_projector, panel_projector_with_source,
};
use crate::core::{
    AccumulatorConfig, EngineError, RunTaskSpec, SampleErrorProjection, SampleStopCondition,
};
use crate::evaluation::{
    Accumulator, AccumulatorState, GammaLoopDiagnostics, Point, SemanticAccumulatorKind,
};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelState, PanelWidth, PlotPoint, TableStateOptions,
    TickBreakdownSegment, key_value, key_value_panel, panel_spec, progress_panel,
    scalar_timeseries_panel_with_smoothing, table_panel_with_payload,
    table_panel_with_payload_and_options, tick_breakdown_panel, with_panel_width,
};
use gammalooprs::observables::{ObservablePhase, ObservableValueTransform};
use serde_json::Value as JsonValue;
use serde_json::json;

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
    if matches!(
        accumulator_config,
        Some(AccumulatorConfig::Scalar | AccumulatorConfig::Complex | AccumulatorConfig::Gammaloop)
            | None
    ) {
        projectors.push(max_weight_summary_projector(accumulator_config.as_ref()));
        projectors.push(max_weight_points_projector(accumulator_config.as_ref()));
    }
    projectors.push(rsd_history_projector(accumulator_config.as_ref()));
    if matches!(
        accumulator_config,
        Some(AccumulatorConfig::Gammaloop) | None
    ) {
        projectors.push(gammaloop_histogram_bundle_projector());
        projectors.push(gammaloop_evaluation_timing_projector());
        projectors.push(gammaloop_evaluation_diagnostics_projector());
    }
    projectors
}

fn max_weight_summary_projector(
    accumulator_config: Option<&AccumulatorConfig>,
) -> TaskPanelProjector {
    let accumulator_config = accumulator_config.cloned();
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "max_weight_summary",
                "Max Weight Impact",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Half,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            Ok(sample_accumulator(ctx, accumulator_config.as_ref())?
                .and_then(max_weight_summary_panel))
        },
        |_ctx| Ok(None),
    )
}

fn max_weight_points_projector(
    accumulator_config: Option<&AccumulatorConfig>,
) -> TaskPanelProjector {
    let accumulator_config = accumulator_config.cloned();
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "max_weight_points",
                "Max Weight Points",
                PanelKind::Table,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            Ok(sample_accumulator(ctx, accumulator_config.as_ref())?
                .and_then(max_weight_points_panel))
        },
        |_ctx| Ok(None),
    )
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
            let eta_seconds = sample_eta_seconds(ctx)?;
            Ok(Some(progress_panel(
                "sample_progress",
                current,
                ctx.task
                    .task
                    .nr_expected_samples()
                    .map(|value| value as f64),
                Some("samples"),
                eta_seconds,
            )))
        },
        |_ctx| Ok(None),
    )
}

fn real_estimate_history_projector(
    accumulator_config: Option<&AccumulatorConfig>,
) -> TaskPanelProjector {
    let current_config = accumulator_config.cloned();
    let history_config = accumulator_config.cloned();
    let width = if matches!(
        accumulator_config,
        Some(AccumulatorConfig::Complex | AccumulatorConfig::Gammaloop)
    ) {
        PanelWidth::Half
    } else {
        PanelWidth::Full
    };
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "real_estimate_history",
                estimate_label(accumulator_config),
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            width,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let target = run_target_from_json(ctx.run_target).map(|target| target.re);
            Ok(sample_accumulator(ctx, current_config.as_ref())?
                .and_then(|accumulator| Some(real_estimate_history_panel(accumulator)))
                .map(|panel| with_scalar_target(panel, target)))
        },
        move |ctx| {
            Ok(decode_history_observable(ctx, history_config.as_ref())?
                .and_then(|accumulator| Some(real_estimate_history_panel(accumulator))))
        },
    )
}

fn imag_estimate_history_projector(
    accumulator_config: Option<&AccumulatorConfig>,
) -> TaskPanelProjector {
    let current_config = accumulator_config.cloned();
    let history_config = accumulator_config.cloned();
    panel_projector_with_source(
        with_panel_width(
            panel_spec(
                "imag_estimate_history",
                "Imaginary Mean",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Half,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let target = run_target_from_json(ctx.run_target).map(|target| target.im);
            Ok(sample_accumulator(ctx, current_config.as_ref())?
                .and_then(imag_estimate_history_panel)
                .map(|panel| with_scalar_target(panel, target)))
        },
        move |ctx| {
            Ok(decode_history_observable(ctx, history_config.as_ref())?
                .and_then(imag_estimate_history_panel))
        },
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
            PanelWidth::Full,
        ),
        TaskPanelCurrentSourcePolicy::PersistedFirst,
        move |ctx| {
            let run_target = run_target_from_json(ctx.run_target);
            Ok(sample_accumulator(ctx, accumulator_config.as_ref())?
                .map(|accumulator| estimate_summary_panel(accumulator, run_target)))
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

fn gammaloop_evaluation_diagnostics_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                "gammaloop_evaluation_diagnostics",
                "Evaluation Diagnostics",
                PanelKind::KeyValue,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            Ok(
                sample_accumulator(ctx, Some(&AccumulatorConfig::Gammaloop))?
                    .and_then(gammaloop_evaluation_diagnostics_panel),
            )
        },
        |ctx| {
            Ok(
                decode_history_observable(ctx, Some(&AccumulatorConfig::Gammaloop))?
                    .and_then(gammaloop_evaluation_diagnostics_panel),
            )
        },
    )
}

fn gammaloop_evaluation_timing_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                "gammaloop_evaluation_timing",
                "Evaluation Timing Breakdown",
                PanelKind::TickBreakdown,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            Ok(
                sample_accumulator(ctx, Some(&AccumulatorConfig::Gammaloop))?
                    .and_then(gammaloop_evaluation_timing_panel),
            )
        },
        |ctx| {
            Ok(
                decode_history_observable(ctx, Some(&AccumulatorConfig::Gammaloop))?
                    .and_then(gammaloop_evaluation_timing_panel),
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
    run_target: Option<RunTarget>,
) -> PanelState {
    key_value_panel(
        "estimate_summary",
        base_estimate_summary_entries(&accumulator, run_target),
    )
}

fn max_weight_summary_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let entries = match accumulator {
        AccumulatorState::Scalar(state) => vec![
            key_value("max_weight_impact", "Impact", state.max_weight_impact()),
            key_value(
                "max_weighted_positive",
                "Max +",
                state.max_weighted_positive,
            ),
            key_value(
                "max_weighted_negative",
                "Max -",
                state.max_weighted_negative,
            ),
        ],
        AccumulatorState::Complex(state) => vec![
            key_value("max_weight_impact", "Impact", state.max_weight_impact()),
            key_value(
                "max_weight_impact_real",
                "Impact Re",
                state.real_max_weight_impact(),
            ),
            key_value(
                "max_weight_impact_imag",
                "Impact Im",
                state.imag_max_weight_impact(),
            ),
            key_value(
                "max_real_weighted_positive",
                "Max Re +",
                state.max_real_weighted_positive,
            ),
            key_value(
                "max_real_weighted_negative",
                "Max Re -",
                state.max_real_weighted_negative,
            ),
            key_value(
                "max_imag_weighted_positive",
                "Max Im +",
                state.max_imag_weighted_positive,
            ),
            key_value(
                "max_imag_weighted_negative",
                "Max Im -",
                state.max_imag_weighted_negative,
            ),
        ],
        AccumulatorState::Gammaloop(state) => {
            let estimate = &state.estimate;
            vec![
                key_value("max_weight_impact", "Impact", estimate.max_weight_impact()),
                key_value(
                    "max_weight_impact_real",
                    "Impact Re",
                    estimate.real_max_weight_impact(),
                ),
                key_value(
                    "max_weight_impact_imag",
                    "Impact Im",
                    estimate.imag_max_weight_impact(),
                ),
                key_value(
                    "max_real_weighted_positive",
                    "Max Re +",
                    estimate.max_real_weighted_positive,
                ),
                key_value(
                    "max_real_weighted_negative",
                    "Max Re -",
                    estimate.max_real_weighted_negative,
                ),
                key_value(
                    "max_imag_weighted_positive",
                    "Max Im +",
                    estimate.max_imag_weighted_positive,
                ),
                key_value(
                    "max_imag_weighted_negative",
                    "Max Im -",
                    estimate.max_imag_weighted_negative,
                ),
            ]
        }
        _ => return None,
    };
    Some(key_value_panel("max_weight_summary", entries))
}

fn max_weight_points_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let columns = vec![
        "Component".to_string(),
        "Sign".to_string(),
        "Integrand".to_string(),
        "Jacobian".to_string(),
        "Max Weighted Value".to_string(),
        "Impact".to_string(),
        "Point".to_string(),
    ];
    let rows = match accumulator {
        AccumulatorState::Scalar(state) => {
            let impact = state.max_weight_impact();
            let mut rows = Vec::new();
            push_max_weight_row(
                &mut rows,
                "scalar",
                "+",
                state.max_weighted_positive,
                impact,
                state.max_weighted_positive_point.as_ref(),
            );
            push_max_weight_row(
                &mut rows,
                "scalar",
                "-",
                state.max_weighted_negative,
                impact,
                state.max_weighted_negative_point.as_ref(),
            );
            rows
        }
        AccumulatorState::Complex(state) => {
            let mut rows = Vec::new();
            push_max_weight_row(
                &mut rows,
                "re",
                "+",
                state.max_real_weighted_positive,
                state.real_max_weight_impact(),
                state.max_real_weighted_positive_point.as_ref(),
            );
            push_max_weight_row(
                &mut rows,
                "re",
                "-",
                state.max_real_weighted_negative,
                state.real_max_weight_impact(),
                state.max_real_weighted_negative_point.as_ref(),
            );
            push_max_weight_row(
                &mut rows,
                "im",
                "+",
                state.max_imag_weighted_positive,
                state.imag_max_weight_impact(),
                state.max_imag_weighted_positive_point.as_ref(),
            );
            push_max_weight_row(
                &mut rows,
                "im",
                "-",
                state.max_imag_weighted_negative,
                state.imag_max_weight_impact(),
                state.max_imag_weighted_negative_point.as_ref(),
            );
            rows
        }
        AccumulatorState::Gammaloop(state) => {
            let estimate = &state.estimate;
            let mut rows = Vec::new();
            push_max_weight_row(
                &mut rows,
                "re",
                "+",
                estimate.max_real_weighted_positive,
                estimate.real_max_weight_impact(),
                estimate.max_real_weighted_positive_point.as_ref(),
            );
            push_max_weight_row(
                &mut rows,
                "re",
                "-",
                estimate.max_real_weighted_negative,
                estimate.real_max_weight_impact(),
                estimate.max_real_weighted_negative_point.as_ref(),
            );
            push_max_weight_row(
                &mut rows,
                "im",
                "+",
                estimate.max_imag_weighted_positive,
                estimate.imag_max_weight_impact(),
                estimate.max_imag_weighted_positive_point.as_ref(),
            );
            push_max_weight_row(
                &mut rows,
                "im",
                "-",
                estimate.max_imag_weighted_negative,
                estimate.imag_max_weight_impact(),
                estimate.max_imag_weighted_negative_point.as_ref(),
            );
            rows
        }
        _ => return None,
    };
    Some(table_panel_with_payload(
        "max_weight_points",
        columns,
        rows,
        None,
    ))
}

fn push_max_weight_row(
    rows: &mut Vec<Vec<JsonValue>>,
    component: &str,
    sign: &str,
    max_weighted_value: f64,
    impact: f64,
    point: Option<&Point>,
) {
    if point.is_none() && max_weighted_value == 0.0 {
        return;
    }
    let (integrand, jacobian) = integrand_and_jacobian_for_component(point, component);
    rows.push(vec![
        JsonValue::String(component.to_string()),
        JsonValue::String(sign.to_string()),
        json_number_or_na(integrand),
        json_number_or_na(jacobian),
        JsonValue::from(max_weighted_value),
        JsonValue::from(impact),
        JsonValue::String(format_point(point)),
    ]);
}

fn integrand_and_jacobian_for_component(
    point: Option<&Point>,
    component: &str,
) -> (Option<f64>, Option<f64>) {
    let Some(point) = point else {
        return (None, None);
    };
    let integrand = match component {
        "re" => point.integrand_value_re,
        "im" => point.integrand_value_im,
        "scalar" => point.integrand_value_re,
        _ => None,
    };
    let jacobian = point
        .factor_product_matching(|label| label.contains("jacobian"))
        .or(point.parameterization_jacobian);
    (integrand, jacobian)
}

fn json_number_or_na(value: Option<f64>) -> JsonValue {
    match value {
        Some(number) if number.is_finite() => JsonValue::from(number),
        _ => JsonValue::String("n/a".to_string()),
    }
}

fn format_point(point: Option<&Point>) -> String {
    match point {
        Some(point) => format!(
            "d={:?}, c={:?}, w={:+.6e}",
            point.discrete,
            point.continuous,
            point.total_weight()
        ),
        None => "N/A".to_string(),
    }
}

fn base_estimate_summary_entries(
    accumulator: &AccumulatorState,
    run_target: Option<RunTarget>,
) -> Vec<crate::server::panels::KeyValueEntry> {
    match accumulator {
        AccumulatorState::Empty(_) => vec![key_value("count", "Count", 0)],
        AccumulatorState::Scalar(state) => vec![
            key_value("count", "Count", state.count),
            key_value("rsd", "RSD", state.rsd()),
            key_value(
                "mean",
                "Mean",
                json!({"kind":"estimate","value":state.mean(),"error":state.stderr()}),
            ),
            key_value(
                "mean_abs",
                "Mean Abs",
                json!({"kind":"estimate","value":state.mean_abs(),"error":state.stderr()}),
            ),
            key_value(
                "signal_to_noise",
                "Mean(|x|)^2 / abs_err^2",
                state.signal_to_noise(),
            ),
        ],
        AccumulatorState::Complex(state) => vec![
            key_value("count", "Count", state.count),
            key_value("rsd", "RSD", state.rsd()),
            key_value(
                "real_mean",
                "Real Mean",
                json!({"kind":"estimate","value":state.real_mean(),"error":state.real_stderr()}),
            ),
            key_value(
                "imag_mean",
                "Imag Mean",
                json!({"kind":"estimate","value":state.imag_mean(),"error":state.imag_stderr()}),
            ),
            key_value(
                "abs_mean",
                "Abs Mean",
                json!({"kind":"estimate","value":state.abs_mean(),"error":state.abs_stderr()}),
            ),
            key_value(
                "signal_to_noise",
                "Mean(|x|)^2 / abs_err^2",
                state.signal_to_noise(),
            ),
        ],
        AccumulatorState::Gammaloop(state) => {
            let mut entries = vec![
                key_value("count", "Count", state.sample_count()),
                key_value("rsd", "RSD", state.rsd()),
                key_value(
                    "real_mean",
                    "Real Mean",
                    json!({"kind":"estimate","value":state.real_mean(),"error":state.real_stderr()}),
                ),
                key_value(
                    "imag_mean",
                    "Imag Mean",
                    json!({"kind":"estimate","value":state.imag_mean(),"error":state.imag_stderr()}),
                ),
            ];
            if let Some(target) = run_target {
                entries.push(key_value(
                    "target_comparison_real",
                    "Real vs Target",
                    json!({
                        "kind":"target_comparison",
                        "value": state.real_mean(),
                        "error": state.real_stderr(),
                        "target": target.re,
                        "delta_percent": delta_percent(state.real_mean(), target.re),
                        "delta_sigma": delta_sigma(state.real_mean(), state.real_stderr(), target.re),
                    }),
                ));
                entries.push(key_value(
                    "target_comparison_imag",
                    "Imag vs Target",
                    json!({
                        "kind":"target_comparison",
                        "value": state.imag_mean(),
                        "error": state.imag_stderr(),
                        "target": target.im,
                        "delta_percent": delta_percent(state.imag_mean(), target.im),
                        "delta_sigma": delta_sigma(state.imag_mean(), state.imag_stderr(), target.im),
                    }),
                ));
            }
            entries.push(key_value(
                "abs_mean",
                "Abs Mean",
                json!({"kind":"estimate","value":state.abs_mean(),"error":state.abs_stderr()}),
            ));
            entries
        }
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

#[derive(Debug, Clone, Copy)]
struct RunTarget {
    re: f64,
    im: f64,
}

fn run_target_from_json(run_target: Option<&JsonValue>) -> Option<RunTarget> {
    let value = run_target?;
    if let Some(scalar) = value.as_f64() {
        return Some(RunTarget {
            re: scalar,
            im: 0.0,
        });
    }
    let object = value.as_object()?;
    let kind = object
        .get("kind")
        .or_else(|| object.get("type"))
        .and_then(JsonValue::as_str)
        .map(|value| value.to_ascii_lowercase());
    if matches!(kind.as_deref(), Some("scalar") | Some("value")) {
        let scalar = object.get("value").and_then(JsonValue::as_f64)?;
        return Some(RunTarget {
            re: scalar,
            im: 0.0,
        });
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
    Some(RunTarget { re, im })
}

fn with_scalar_target(panel: PanelState, target: Option<f64>) -> PanelState {
    match panel {
        PanelState::ScalarTimeseries {
            panel_id,
            points,
            smooth,
            ..
        } => PanelState::ScalarTimeseries {
            panel_id,
            points,
            smooth,
            target,
        },
        other => other,
    }
}

fn delta_sigma(value: f64, error: f64, target: f64) -> f64 {
    let sigma = error.abs();
    if sigma > 0.0 && sigma.is_finite() && value.is_finite() && target.is_finite() {
        (value - target).abs() / sigma
    } else if (value - target).abs() == 0.0 {
        0.0
    } else {
        f64::INFINITY
    }
}

fn delta_percent(value: f64, target: f64) -> f64 {
    let denominator = target.abs();
    if denominator > 0.0 && denominator.is_finite() && value.is_finite() && target.is_finite() {
        (value - target).abs() / denominator * 100.0
    } else if (value - target).abs() == 0.0 {
        0.0
    } else {
        f64::INFINITY
    }
}

fn sample_eta_seconds(ctx: &TaskPanelContext<'_>) -> Result<Option<f64>, EngineError> {
    let Some(stop_condition) = ctx.task.task.sample_stop_condition() else {
        return Ok(None);
    };
    if let Some(smoothed_eta_seconds) = ctx.smoothed_eta_seconds {
        return Ok(Some(smoothed_eta_seconds));
    }
    let accumulator = sample_accumulator(ctx, None)?;
    let projection = stop_condition
        .projection
        .unwrap_or_else(|| match accumulator {
            Some(AccumulatorState::Complex(_) | AccumulatorState::Gammaloop(_)) => {
                SampleErrorProjection::Abs
            }
            _ => SampleErrorProjection::Real,
        });
    let projected = accumulator
        .as_ref()
        .and_then(|accumulator| projected_estimate(accumulator, projection));
    let eta_seconds = estimate_eta_seconds(
        stop_condition,
        projected,
        ctx.task.nr_completed_samples,
        ctx.completed_samples_per_second,
    );
    Ok(eta_seconds)
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

fn gammaloop_histogram_bundle_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let AccumulatorState::Gammaloop(state) = accumulator else {
        return None;
    };
    let payload = serde_json::to_value(&state.bundle).unwrap_or(JsonValue::Null);

    let columns = vec![
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
    ];
    let rows = state
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
        .collect::<Vec<_>>();
    let name_column_index = columns.iter().position(|column| column == "Name");
    let hidden_columns = ["Name", "Log X", "Log Y"];
    let visible_column_indices = columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| {
            if hidden_columns.iter().any(|hidden| column == hidden) {
                None
            } else {
                Some(index)
            }
        })
        .collect::<Vec<_>>();
    let row_keys = name_column_index.and_then(|index| {
        rows.iter()
            .map(|row| match row.get(index) {
                Some(JsonValue::String(name)) => Some(name.clone()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
    });

    Some(table_panel_with_payload_and_options(
        "gammaloop_histogram_bundle",
        columns,
        rows,
        Some(payload),
        TableStateOptions {
            visible_column_indices,
            row_keys,
        },
    ))
}

fn gammaloop_evaluation_diagnostics_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let AccumulatorState::Gammaloop(state) = accumulator else {
        return None;
    };
    Some(key_value_panel(
        "gammaloop_evaluation_diagnostics",
        gammaloop_diagnostics_entries(&state.diagnostics),
    ))
}

fn gammaloop_diagnostics_entries(
    diagnostics: &GammaLoopDiagnostics,
) -> Vec<crate::server::panels::KeyValueEntry> {
    vec![
        key_value("count_total", "Evaluations", diagnostics.count_total),
        key_value(
            "count_double_precision",
            "Double Precision",
            diagnostics.count_double_precision,
        ),
        key_value(
            "count_quad_precision",
            "Quad Precision",
            diagnostics.count_quad_precision,
        ),
        key_value(
            "count_arb_precision",
            "Arb Precision",
            diagnostics.count_arb_precision,
        ),
        key_value(
            "promoted_to_quad_ratio",
            "Quad Fraction",
            diagnostics.promoted_to_quad_ratio(),
        ),
        key_value(
            "promoted_to_arb_ratio",
            "Arb Fraction",
            diagnostics.promoted_to_arb_ratio(),
        ),
        key_value("count_nan", "NaN", diagnostics.count_nan),
        key_value(
            "count_nan_or_unstable",
            "NaN Or Unstable",
            diagnostics.count_nan_or_unstable,
        ),
        key_value(
            "nan_or_unstable_ratio",
            "NaN/Unstable Fraction",
            diagnostics.nan_or_unstable_ratio(),
        ),
        key_value(
            "count_loop_momenta_escalated",
            "Loop Momenta Escalated",
            diagnostics.count_loop_momenta_escalated,
        ),
        key_value(
            "loop_momenta_escalated_ratio",
            "Escalation Fraction",
            diagnostics.loop_momenta_escalated_ratio(),
        ),
        key_value(
            "total_generated_events",
            "Generated Events",
            diagnostics.total_generated_events,
        ),
        key_value(
            "total_accepted_events",
            "Accepted Events",
            diagnostics.total_accepted_events,
        ),
        key_value(
            "accepted_event_ratio",
            "Accepted/Generated",
            diagnostics.accepted_event_ratio(),
        ),
    ]
}

fn gammaloop_evaluation_timing_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let AccumulatorState::Gammaloop(state) = accumulator else {
        return None;
    };
    let diagnostics = &state.diagnostics;
    let total_eval_ms = diagnostics.avg_eval_time_ms();
    if !total_eval_ms.is_finite() || total_eval_ms <= 0.0 {
        return None;
    }

    let raw_parameterization = diagnostics.avg_parameterization_time_ms().max(0.0);
    let raw_integrand = diagnostics.avg_integrand_eval_time_ms().max(0.0);
    let raw_evaluator = diagnostics.avg_evaluator_eval_time_ms().max(0.0);
    let raw_events = diagnostics.avg_event_processing_time_ms().max(0.0);

    // GammaLoop timings are flamegraph-style: evaluator/event are nested inside
    // integrand. Keep only the explicitly reported categories here.
    let parameterization = raw_parameterization.min(total_eval_ms);
    let remaining_after_parameterization = (total_eval_ms - parameterization).max(0.0);
    let integrand = raw_integrand.min(remaining_after_parameterization);

    let nested_sum = raw_evaluator + raw_events;
    let nested_scale = if nested_sum > 0.0 {
        (integrand / nested_sum).min(1.0)
    } else {
        0.0
    };
    let evaluator = raw_evaluator * nested_scale;
    let events = raw_events * nested_scale;
    let integrand_core = (integrand - evaluator - events).max(0.0);

    let segments = vec![
        timing_segment(
            "parameterization",
            "Parameterization",
            parameterization,
            "#0a9396",
        ),
        timing_segment(
            "integrand_core",
            "Integrand Core",
            integrand_core,
            "#ca6702",
        ),
        timing_segment("evaluator", "Evaluator Call", evaluator, "#bb3e03"),
        timing_segment("events", "Event Processing", events, "#6d597a"),
    ]
    .into_iter()
    .filter(|segment| segment.value_ms.is_finite() && segment.value_ms > 0.0)
    .collect::<Vec<_>>();

    if segments.is_empty() {
        return None;
    }
    let segment_sum_ms: f64 = segments.iter().map(|segment| segment.value_ms).sum();
    if !segment_sum_ms.is_finite() || segment_sum_ms <= 0.0 {
        return None;
    }

    Some(tick_breakdown_panel(
        "gammaloop_evaluation_timing",
        segment_sum_ms,
        segments,
    ))
}

fn timing_segment(key: &str, label: &str, value_ms: f64, color: &str) -> TickBreakdownSegment {
    TickBreakdownSegment {
        key: key.to_string(),
        label: label.to_string(),
        value_ms,
        color: color.to_string(),
    }
}
