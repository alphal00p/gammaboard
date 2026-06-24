use super::{
    TaskPanelContext, TaskPanelCurrentSourcePolicy, TaskPanelHistoryContext, TaskPanelProjector,
    panel_projector, panel_projector_with_source,
};
use crate::core::{
    AccumulatorConfig, DiscreteProjectionConfig, DiscreteProjectionNormalization, EngineError,
    NamedDiscreteProjection, SampleErrorProjection, SampleStopCondition,
};
use crate::evaluation::accumulator::DiscreteProjectionBinState;
use crate::evaluation::{
    Accumulator, AccumulatorState, GammaLoopDiagnostics, Point, SemanticAccumulatorKind,
    extract_accumulator_metric_with_runtime,
};
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelState, PanelWidth, PlotPoint, TableStateOptions,
    TickBreakdownSegment, key_value, key_value_panel, panel_spec, progress_panel,
    scalar_timeseries_panel_with_smoothing, table_panel_with_payload,
    table_panel_with_payload_and_options, tick_breakdown_panel, with_panel_width,
};
#[cfg(feature = "gammaloop")]
use gammalooprs::observables::{ObservablePhase, ObservableValueTransform};
use serde_json::Value as JsonValue;
use serde_json::json;
use std::collections::BTreeMap;

pub(super) fn projectors(
    effective_accumulator_config: AccumulatorConfig,
) -> Vec<TaskPanelProjector> {
    let accumulator_config = effective_accumulator_config;
    let mut projectors = vec![
        sample_progress_projector(&accumulator_config),
        estimate_summary_projector(&accumulator_config),
        real_estimate_history_projector(&accumulator_config),
    ];
    if matches!(accumulator_config, AccumulatorConfig::Gammaloop) {
        projectors.push(imag_estimate_history_projector(&accumulator_config));
    }
    if matches!(
        accumulator_config,
        AccumulatorConfig::Scalar { .. }
            | AccumulatorConfig::Vector { .. }
            | AccumulatorConfig::Gammaloop
    ) {
        projectors.push(max_weight_summary_projector(&accumulator_config));
        projectors.push(max_weight_points_projector(&accumulator_config));
    }
    projectors.push(rsd_history_projector(&accumulator_config));
    if matches!(accumulator_config, AccumulatorConfig::Gammaloop) {
        #[cfg(feature = "gammaloop")]
        projectors.push(gammaloop_histogram_bundle_projector());
        projectors.push(gammaloop_evaluation_timing_projector());
        projectors.push(gammaloop_evaluation_diagnostics_projector());
    } else if let Some(projection_config) = accumulator_config.discrete_projections().cloned()
        && matches!(
            accumulator_config,
            AccumulatorConfig::Scalar { .. } | AccumulatorConfig::Vector { .. }
        )
    {
        projectors.push(discrete_projection_bundle_projector(
            accumulator_config,
            projection_config,
        ));
    }
    projectors
}

fn max_weight_summary_projector(accumulator_config: &AccumulatorConfig) -> TaskPanelProjector {
    let accumulator_config = accumulator_config.clone();
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
            Ok(sample_accumulator(ctx, &accumulator_config)?.and_then(max_weight_summary_panel))
        },
        |_ctx| Ok(None),
    )
}

fn max_weight_points_projector(accumulator_config: &AccumulatorConfig) -> TaskPanelProjector {
    let accumulator_config = accumulator_config.clone();
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
            Ok(sample_accumulator(ctx, &accumulator_config)?.and_then(max_weight_points_panel))
        },
        |_ctx| Ok(None),
    )
}

fn sample_progress_projector(accumulator_config: &AccumulatorConfig) -> TaskPanelProjector {
    let accumulator_config = accumulator_config.clone();
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
        move |ctx| {
            let current = sample_progress_value(ctx);
            let eta_seconds = sample_eta_seconds(ctx, &accumulator_config)?;
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

fn real_estimate_history_projector(accumulator_config: &AccumulatorConfig) -> TaskPanelProjector {
    let current_config = accumulator_config.clone();
    let history_config = accumulator_config.clone();
    let width = if matches!(accumulator_config, AccumulatorConfig::Gammaloop) {
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
            let target = run_target_from_json(ctx.run_target)
                .and_then(|target| target.component(&["real", "value"]));
            Ok(sample_accumulator(ctx, &current_config)?
                .and_then(real_estimate_history_panel)
                .map(|panel| with_scalar_target(panel, target)))
        },
        move |ctx| {
            Ok(decode_history_observable(ctx, &history_config)?
                .and_then(real_estimate_history_panel))
        },
    )
}

fn imag_estimate_history_projector(accumulator_config: &AccumulatorConfig) -> TaskPanelProjector {
    let current_config = accumulator_config.clone();
    let history_config = accumulator_config.clone();
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
            let target =
                run_target_from_json(ctx.run_target).and_then(|target| target.component(&["imag"]));
            Ok(sample_accumulator(ctx, &current_config)?
                .and_then(imag_estimate_history_panel)
                .map(|panel| with_scalar_target(panel, target)))
        },
        move |ctx| {
            Ok(decode_history_observable(ctx, &history_config)?
                .and_then(imag_estimate_history_panel))
        },
    )
}

fn rsd_history_projector(accumulator_config: &AccumulatorConfig) -> TaskPanelProjector {
    persisted_first_history_projector(
        "abs_signal_to_noise_history",
        "RSD",
        accumulator_config.clone(),
        rsd_history_panel,
    )
}

fn persisted_first_history_projector<F>(
    panel_id: &'static str,
    label: &'static str,
    accumulator_config: AccumulatorConfig,
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
        move |ctx| Ok(sample_accumulator(ctx, &current_config)?.and_then(map_panel)),
        move |ctx| Ok(decode_history_observable(ctx, &history_config)?.and_then(map_panel)),
    )
}

fn estimate_summary_projector(accumulator_config: &AccumulatorConfig) -> TaskPanelProjector {
    let accumulator_config = accumulator_config.clone();
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
            Ok(sample_accumulator(ctx, &accumulator_config)?
                .map(|accumulator| estimate_summary_panel(accumulator, run_target)))
        },
        |_ctx| Ok(None),
    )
}

#[cfg(feature = "gammaloop")]
fn gammaloop_histogram_bundle_projector() -> TaskPanelProjector {
    panel_projector(
        with_panel_width(
            panel_spec(
                "gammaloop_histogram_bundle",
                "GammaLoop Histograms",
                PanelKind::Table,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        |ctx| {
            Ok(sample_accumulator(ctx, &AccumulatorConfig::Gammaloop)?
                .and_then(gammaloop_histogram_bundle_panel))
        },
        |ctx| {
            Ok(
                decode_history_observable(ctx, &AccumulatorConfig::Gammaloop)?
                    .and_then(gammaloop_histogram_bundle_panel),
            )
        },
    )
}

fn discrete_projection_bundle_projector(
    accumulator_config: AccumulatorConfig,
    projection_config: DiscreteProjectionConfig,
) -> TaskPanelProjector {
    let current_accumulator_config = accumulator_config.clone();
    let history_accumulator_config = accumulator_config;
    let current_projection_config = projection_config.clone();
    let history_projection_config = projection_config;
    panel_projector(
        with_panel_width(
            panel_spec(
                "discrete_projection_bundle",
                "Discrete Projections",
                PanelKind::Table,
                PanelHistoryMode::None,
            ),
            PanelWidth::Full,
        ),
        move |ctx| {
            Ok(
                sample_accumulator(ctx, &current_accumulator_config)?.and_then(|accumulator| {
                    discrete_projection_bundle_panel(
                        accumulator,
                        &current_projection_config,
                        ctx.sampler_engine_diagnostics,
                    )
                }),
            )
        },
        move |ctx| {
            Ok(
                decode_history_observable(ctx, &history_accumulator_config)?.and_then(
                    |accumulator| {
                        discrete_projection_bundle_panel(
                            accumulator,
                            &history_projection_config,
                            None,
                        )
                    },
                ),
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
            Ok(sample_accumulator(ctx, &AccumulatorConfig::Gammaloop)?
                .and_then(gammaloop_evaluation_diagnostics_panel))
        },
        |ctx| {
            Ok(
                decode_history_observable(ctx, &AccumulatorConfig::Gammaloop)?
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
            Ok(sample_accumulator(ctx, &AccumulatorConfig::Gammaloop)?
                .and_then(gammaloop_evaluation_timing_panel))
        },
        |ctx| {
            Ok(
                decode_history_observable(ctx, &AccumulatorConfig::Gammaloop)?
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
    accumulator_config: &AccumulatorConfig,
) -> Result<Option<AccumulatorState>, EngineError> {
    if let Some(accumulator) = ctx.source.accumulator() {
        if accumulator_matches_requested_config(accumulator, accumulator_config) {
            return Ok(Some(accumulator.clone()));
        }
        if ctx.source.persisted().is_none() {
            return Err(EngineError::build(format!(
                "accumulator type mismatch: expected {}, got {} and no persisted snapshot was available",
                config_label(accumulator_config),
                accumulator.kind_str()
            )));
        }
    }
    match ctx.source.persisted() {
        Some(persisted) => {
            decode_aggregate_persisted_accumulator(accumulator_config, persisted).map(Some)
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
        AccumulatorConfig::Scalar { .. } => matches!(
            accumulator,
            AccumulatorState::Scalar(_) | AccumulatorState::FullVector(_)
        ),
        AccumulatorConfig::Vector { .. } => matches!(accumulator, AccumulatorState::Vector(_)),
        AccumulatorConfig::Gammaloop => matches!(accumulator, AccumulatorState::Gammaloop(_)),
        AccumulatorConfig::FullVector { .. } => {
            matches!(accumulator, AccumulatorState::FullVector(_))
        }
    }
}

fn decode_history_observable(
    ctx: &TaskPanelHistoryContext<'_>,
    accumulator_config: &AccumulatorConfig,
) -> Result<Option<AccumulatorState>, EngineError> {
    decode_aggregate_persisted_accumulator(accumulator_config, &ctx.snapshot.persisted_output)
        .map(Some)
}

fn decode_aggregate_persisted_accumulator(
    config: &AccumulatorConfig,
    persisted: &JsonValue,
) -> Result<AccumulatorState, EngineError> {
    match config {
        AccumulatorConfig::Empty => Err(EngineError::build(
            "sample task expected aggregate accumulator, got empty".to_string(),
        )),
        AccumulatorConfig::Scalar { .. } => AccumulatorState::from_aggregate_persistent_json(
            SemanticAccumulatorKind::Scalar,
            persisted,
        ),
        AccumulatorConfig::Vector { .. } => {
            AccumulatorState::from_vector_persistent_json(persisted)
        }
        AccumulatorConfig::Gammaloop => AccumulatorState::from_gammaloop_persistent_json(persisted),
        AccumulatorConfig::FullVector { .. } => Err(EngineError::build(format!(
            "sample task expected aggregate accumulator, got {}",
            config_label(config)
        ))),
    }
}

fn estimate_label(accumulator_config: &AccumulatorConfig) -> &'static str {
    match accumulator_config {
        AccumulatorConfig::Empty => "Estimate",
        AccumulatorConfig::Scalar { .. } => "Mean",
        AccumulatorConfig::Vector { .. } => "Projection Mean",
        AccumulatorConfig::Gammaloop => "Real Mean",
        AccumulatorConfig::FullVector { .. } => "Estimate",
    }
}

fn config_label(config: &AccumulatorConfig) -> &'static str {
    match config {
        AccumulatorConfig::Empty => "empty",
        AccumulatorConfig::Scalar { .. } => "scalar",
        AccumulatorConfig::Vector { .. } => "vector",
        AccumulatorConfig::Gammaloop => "gammaloop",
        AccumulatorConfig::FullVector { .. } => "full_vector",
    }
}

fn real_estimate_history_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    if accumulator.sample_count() <= 0 {
        return None;
    }
    let smooth = Some(true);
    Some(match accumulator {
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
        AccumulatorState::Vector(state) => {
            let projection = &state.projection.state;
            scalar_timeseries_panel_with_smoothing(
                "real_estimate_history",
                vec![PlotPoint {
                    x: projection.count as f64,
                    y: projection.mean(),
                    x_sampler_uptime_ms: None,
                    x_completed_samples_total: None,
                    y_min: Some(projection.mean() - projection.stderr()),
                    y_max: Some(projection.mean() + projection.stderr()),
                }],
                smooth,
            )
        }
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
    })
}

fn imag_estimate_history_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    if accumulator.sample_count() <= 0 {
        return None;
    }
    let smooth = Some(true);
    match accumulator {
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

fn rsd_history_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    if accumulator.sample_count() <= 0 {
        return None;
    }
    let rsd = match &accumulator {
        AccumulatorState::Scalar(state) => state.rsd(),
        AccumulatorState::Vector(state) => state.rsd(),
        AccumulatorState::Gammaloop(state) => state.rsd(),
        _ => 0.0,
    };
    Some(scalar_timeseries_panel_with_smoothing(
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
    ))
}

fn estimate_summary_panel(
    accumulator: AccumulatorState,
    run_target: Option<VectorTarget>,
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
        AccumulatorState::Vector(state) => vec![
            key_value(
                "max_weight_impact",
                "Projection Impact",
                state.projection.state.max_weight_impact(),
            ),
            key_value(
                "max_weighted_positive",
                "Projection Max +",
                state.projection.state.max_weighted_positive,
            ),
            key_value(
                "max_weighted_negative",
                "Projection Max -",
                state.projection.state.max_weighted_negative,
            ),
        ],
        AccumulatorState::Gammaloop(state) => {
            vec![
                key_value(
                    "max_weight_impact",
                    "Projection Impact",
                    state.estimate.projection.state.max_weight_impact(),
                ),
                key_value(
                    "max_weighted_positive",
                    "Projection Max +",
                    state.estimate.projection.state.max_weighted_positive,
                ),
                key_value(
                    "max_weighted_negative",
                    "Projection Max -",
                    state.estimate.projection.state.max_weighted_negative,
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
        AccumulatorState::Vector(state) => {
            let mut rows = Vec::new();
            for component in &state.components {
                let impact = component.state.max_weight_impact();
                push_max_weight_row(
                    &mut rows,
                    &component.name,
                    "+",
                    component.state.max_weighted_positive,
                    impact,
                    component.state.max_weighted_positive_point.as_ref(),
                );
                push_max_weight_row(
                    &mut rows,
                    &component.name,
                    "-",
                    component.state.max_weighted_negative,
                    impact,
                    component.state.max_weighted_negative_point.as_ref(),
                );
            }
            rows
        }
        AccumulatorState::Gammaloop(state) => {
            let estimate = &state.estimate;
            let mut rows = Vec::new();
            for component in &estimate.components {
                let impact = component.state.max_weight_impact();
                push_max_weight_row(
                    &mut rows,
                    &component.name,
                    "+",
                    component.state.max_weighted_positive,
                    impact,
                    component.state.max_weighted_positive_point.as_ref(),
                );
                push_max_weight_row(
                    &mut rows,
                    &component.name,
                    "-",
                    component.state.max_weighted_negative,
                    impact,
                    component.state.max_weighted_negative_point.as_ref(),
                );
            }
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
    run_target: Option<VectorTarget>,
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
        AccumulatorState::Vector(state) => {
            let mut entries = vec![
                key_value("count", "Count", state.sample_count()),
                key_value("projection_rsd", "Projection RSD", state.rsd()),
                key_value(
                    "projection_mean",
                    "Projection Mean",
                    json!({
                        "kind":"estimate",
                        "value":state.projection.state.mean(),
                        "error":state.projection.state.stderr()
                    }),
                ),
                key_value(
                    "projection_signal_to_noise",
                    "Projection Mean(|x|)^2 / abs_err^2",
                    state.signal_to_noise(),
                ),
            ];
            for component in &state.components {
                let key = format!("component_{}_mean", component.name);
                let label = format!("{} Mean", component.name);
                entries.push(key_value(
                    &key,
                    &label,
                    json!({
                        "kind":"estimate",
                        "value":component.state.mean(),
                        "error":component.state.stderr()
                    }),
                ));
            }
            entries
        }
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
                if let Some(real_target) = target.component(&["real", "value"]) {
                    entries.push(key_value(
                        "target_comparison_real",
                        "Real vs Target",
                        json!({
                            "kind":"target_comparison",
                            "value": state.real_mean(),
                            "error": state.real_stderr(),
                            "target": real_target,
                            "delta_percent": delta_percent(state.real_mean(), real_target),
                            "delta_sigma": delta_sigma(state.real_mean(), state.real_stderr(), real_target),
                        }),
                    ));
                }
                if let Some(imag_target) = target.component(&["imag"]) {
                    entries.push(key_value(
                        "target_comparison_imag",
                        "Imag vs Target",
                        json!({
                            "kind":"target_comparison",
                            "value": state.imag_mean(),
                            "error": state.imag_stderr(),
                            "target": imag_target,
                            "delta_percent": delta_percent(state.imag_mean(), imag_target),
                            "delta_sigma": delta_sigma(state.imag_mean(), state.imag_stderr(), imag_target),
                        }),
                    ));
                }
            }
            entries.push(key_value(
                "abs_mean",
                "Abs Mean",
                json!({"kind":"estimate","value":state.abs_mean(),"error":state.abs_stderr()}),
            ));
            entries
        }
        AccumulatorState::FullVector(state) => {
            let mut entries = vec![key_value("count", "Count", state.sample_count())];
            for component in &state.components {
                let values = state.component_values(component).unwrap_or_default();
                entries.push(key_value(
                    &format!("{component}_min"),
                    &format!("{component} Min"),
                    values.iter().copied().fold(f64::INFINITY, f64::min),
                ));
                entries.push(key_value(
                    &format!("{component}_max"),
                    &format!("{component} Max"),
                    values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                ));
            }
            entries
        }
    }
}

#[derive(Debug, Clone)]
struct VectorTarget {
    components: BTreeMap<String, f64>,
}

fn run_target_from_json(run_target: Option<&JsonValue>) -> Option<VectorTarget> {
    let value = run_target?;
    if let Some(scalar) = value.as_f64() {
        return Some(VectorTarget::single("value", scalar));
    }
    let object = value.as_object()?;
    let kind = object
        .get("kind")
        .or_else(|| object.get("type"))
        .and_then(JsonValue::as_str)
        .map(|value| value.to_ascii_lowercase());
    if matches!(kind.as_deref(), Some("scalar") | Some("value")) {
        let scalar = object.get("value").and_then(JsonValue::as_f64)?;
        return Some(VectorTarget::single("value", scalar));
    }
    let source = object
        .get("components")
        .or_else(|| object.get("values"))
        .and_then(JsonValue::as_object)?;
    let components = source
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), value.as_f64()?)))
        .collect::<BTreeMap<_, _>>();
    (!components.is_empty()).then_some(VectorTarget { components })
}

impl VectorTarget {
    fn single(name: &str, value: f64) -> Self {
        Self {
            components: BTreeMap::from([(name.to_string(), value)]),
        }
    }

    fn component(&self, names: &[&str]) -> Option<f64> {
        names
            .iter()
            .find_map(|name| self.components.get(*name).copied())
    }
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

fn sample_eta_seconds(
    ctx: &TaskPanelContext<'_>,
    accumulator_config: &AccumulatorConfig,
) -> Result<Option<f64>, EngineError> {
    let Some(stop_condition) = ctx.task.task.sample_stop_condition() else {
        return Ok(None);
    };
    if let Some(smoothed_eta_seconds) = ctx.smoothed_eta_seconds {
        return Ok(Some(smoothed_eta_seconds));
    }
    let accumulator = sample_accumulator(ctx, accumulator_config)?;
    let projected = if let Some(selector) = &stop_condition.metric {
        accumulator.as_ref().and_then(|accumulator| {
            extract_accumulator_metric_with_runtime(
                accumulator,
                selector,
                ctx.completed_samples_per_second,
            )
            .ok()
            .flatten()
            .and_then(|metric| {
                metric.uncertainty.map(|error| ProjectedEstimate {
                    value: metric.value,
                    error,
                })
            })
        })
    } else {
        let projection = stop_condition.projection.unwrap_or(match accumulator {
            Some(AccumulatorState::Gammaloop(_)) => SampleErrorProjection::Abs,
            _ => SampleErrorProjection::Real,
        });
        accumulator
            .as_ref()
            .and_then(|accumulator| projected_estimate(accumulator, projection))
    };
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
        AccumulatorState::Vector(state) => Some(ProjectedEstimate {
            value: state.projection.state.mean(),
            error: state.projection.state.stderr(),
        }),
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
        AccumulatorState::Empty(_) | AccumulatorState::FullVector(_) => None,
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

#[cfg(feature = "gammaloop")]
fn gammaloop_histogram_bundle_panel(accumulator: AccumulatorState) -> Option<PanelState> {
    let AccumulatorState::Gammaloop(state) = accumulator else {
        return None;
    };
    let mut payload = serde_json::to_value(&state.bundle).unwrap_or(JsonValue::Null);
    if let JsonValue::Object(payload) = &mut payload {
        payload.insert(
            "expands_to".to_string(),
            json!({
                "kind": "histogram",
                "source": "selected_row",
            }),
        );
        payload.insert(
            "actions".to_string(),
            json!({
                "export": true,
                "upload_bundle": true,
            }),
        );
    }

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
        .map(|(name, projection)| {
            vec![
                JsonValue::String(name.clone()),
                JsonValue::String(projection.title.clone()),
                JsonValue::String(match projection.phase {
                    ObservablePhase::Real => "real".to_string(),
                    ObservablePhase::Imag => "imag".to_string(),
                }),
                JsonValue::String(match projection.value_transform {
                    ObservableValueTransform::Identity => "identity".to_string(),
                    ObservableValueTransform::Log10 => "log10".to_string(),
                }),
                JsonValue::from(projection.sample_count as i64),
                JsonValue::from(projection.bins.len() as i64),
                JsonValue::String(match projection.kind {
                    gammalooprs::observables::HistogramSnapshotKind::Continuous => {
                        match (projection.x_min, projection.x_max) {
                            (Some(x_min), Some(x_max)) => format!("[{}, {}]", x_min, x_max),
                            _ => "continuous".to_string(),
                        }
                    }
                    gammalooprs::observables::HistogramSnapshotKind::Discrete => projection
                        .discrete_min_bin_id
                        .map(|min_bin_id| {
                            format!(
                                "[{}, {}]",
                                min_bin_id,
                                min_bin_id + projection.bins.len() as isize
                            )
                        })
                        .unwrap_or_else(|| "discrete".to_string()),
                }),
                JsonValue::from(projection.statistics.in_range_entry_count as i64),
                JsonValue::from(projection.underflow_bin.entry_count as i64),
                JsonValue::from(projection.overflow_bin.entry_count as i64),
                JsonValue::from(projection.statistics.nan_value_count as i64),
                JsonValue::from(projection.statistics.mitigated_pair_count as i64),
                JsonValue::from(projection.supports_misbinning_mitigation),
                JsonValue::from(projection.log_x_axis),
                JsonValue::from(projection.log_y_axis),
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

fn discrete_projection_bundle_panel(
    accumulator: AccumulatorState,
    config: &DiscreteProjectionConfig,
    sampler_engine_diagnostics: Option<&JsonValue>,
) -> Option<PanelState> {
    let discrete_pdf = sampler_engine_diagnostics.and_then(discrete_pdf_cache);
    let payload = match &accumulator {
        AccumulatorState::Scalar(state) => scalar_discrete_projection_payload(
            &state.discrete_bins,
            state.count,
            config,
            discrete_pdf.as_ref(),
        ),
        AccumulatorState::Vector(state) => {
            vector_discrete_projection_payload(state, config, discrete_pdf.as_ref())
        }
        _ => return None,
    };
    let payload = match payload {
        Ok(payload) => payload,
        Err(err) => {
            return Some(key_value_panel(
                "discrete_projection_bundle",
                vec![key_value("error", "Error", err)],
            ));
        }
    };
    let projections = payload
        .get("histograms")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let rows = projections
        .iter()
        .map(|(name, projection)| {
            let bins = projection
                .get("bins")
                .and_then(JsonValue::as_array)
                .map(|bins| bins.len())
                .unwrap_or_default();
            vec![
                JsonValue::String(name.clone()),
                projection
                    .get("title")
                    .and_then(JsonValue::as_str)
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null),
                JsonValue::from(bins as i64),
            ]
        })
        .collect::<Vec<_>>();

    let row_keys = projections.keys().cloned().collect::<Vec<_>>();
    Some(table_panel_with_payload_and_options(
        "discrete_projection_bundle",
        vec!["Name".to_string(), "Title".to_string(), "Bins".to_string()],
        rows,
        Some(payload),
        TableStateOptions {
            visible_column_indices: vec![1, 2],
            row_keys: Some(row_keys),
        },
    ))
}

fn scalar_discrete_projection_payload(
    bins: &BTreeMap<String, DiscreteProjectionBinState>,
    total_count: i64,
    config: &DiscreteProjectionConfig,
    discrete_pdf: Option<&DiscretePdfCache>,
) -> Result<JsonValue, String> {
    let projections = config
        .items
        .iter()
        .map(|item| {
            Ok((
                item.name.clone(),
                json!({
                    "title": item.name,
                    "type_description": discrete_projection_description(item),
                    "metric_descriptors": discrete_projection_metric_descriptors(),
                    "views": discrete_projection_histogram_views(discrete_pdf.is_some()),
                    "controls": discrete_projection_histogram_controls(),
                    "bins": scalar_projected_bins(
                        bins,
                        item,
                        &item.name,
                        config.normalization,
                        total_count,
                        config.bin_limit(),
                        discrete_pdf
                    )?,
                }),
            ))
        })
        .collect::<Result<serde_json::Map<String, JsonValue>, String>>()?;
    Ok(json!({
        "primary_histogram_name": config.items.first().map(|item| item.name.clone()),
        "expands_to": {
            "kind": "histogram",
            "source": "selected_row",
        },
        "actions": {
            "export": false,
            "upload_bundle": false,
        },
        "histograms": projections,
    }))
}

fn vector_discrete_projection_payload(
    state: &crate::evaluation::VectorAccumulatorState,
    config: &DiscreteProjectionConfig,
    discrete_pdf: Option<&DiscretePdfCache>,
) -> Result<JsonValue, String> {
    let mut projections = serde_json::Map::new();
    for component in state
        .components
        .iter()
        .chain(std::iter::once(&state.projection))
    {
        if !discrete_projection_includes_stream(config, &component.name) {
            continue;
        }
        for item in &config.items {
            let name = format!("{}.{}", item.name, component.name);
            projections.insert(
                name.clone(),
                json!({
                    "title": name,
                    "type_description": discrete_projection_description(item),
                    "metric_descriptors": discrete_projection_metric_descriptors(),
                    "views": discrete_projection_histogram_views(discrete_pdf.is_some()),
                    "controls": discrete_projection_histogram_controls(),
                    "bins": scalar_projected_bins(
                        &component.state.discrete_bins,
                        item,
                        &name,
                        config.normalization,
                        component.state.count,
                        config.bin_limit(),
                        discrete_pdf
                    )?,
                }),
            );
        }
    }
    Ok(json!({
        "primary_histogram_name": projections.keys().next().cloned(),
        "expands_to": {
            "kind": "histogram",
            "source": "selected_row",
        },
        "actions": {
            "export": false,
            "upload_bundle": false,
        },
        "histograms": projections,
    }))
}

fn discrete_projection_includes_stream(config: &DiscreteProjectionConfig, stream: &str) -> bool {
    config.streams.is_empty() || config.streams.iter().any(|candidate| candidate == stream)
}

fn discrete_projection_histogram_controls() -> JsonValue {
    json!({
        "scale": true,
        "x_scale": false,
        "pdf_cdf": false,
        "ratio": false,
        "relative_error": true,
        "export": true,
        "reset_view": true,
        "upload_bundle": false,
        "compare_bundles": false,
        "sort": true,
    })
}

fn discrete_projection_metric_descriptors() -> JsonValue {
    json!({
        "value": {
            "label": "Value",
            "short_label": "value",
            "format": "scientific",
        },
        "error": {
            "label": "Absolute Error",
            "short_label": "abs error",
            "format": "scientific",
        },
        "relative_error": {
            "label": "Relative Error",
            "short_label": "rel error",
            "format": "scientific",
        },
        "pdf": {
            "label": "PDF",
            "short_label": "pdf",
            "format": "scientific",
        },
        "pdf_scaled_value": {
            "label": "PDF x <|I|>",
            "short_label": "pdf x <|I|>",
            "format": "scientific",
        },
        "value_pdf_mismatch": {
            "label": "Value - PDF x <|I|>",
            "short_label": "delta",
            "format": "scientific",
        },
        "value_pdf_mismatch_abs": {
            "label": "|Value - PDF x <|I|>|",
            "short_label": "|delta|",
            "format": "scientific",
        },
        "contribution": {
            "label": "Contribution",
            "short_label": "contribution",
            "format": "scientific",
        },
        "conditional_mean": {
            "label": "Conditional Mean",
            "short_label": "conditional mean",
            "format": "scientific",
        },
        "contribution_fraction": {
            "label": "Contribution / <|I|>",
            "short_label": "contribution / <|I|>",
            "format": "scientific",
        },
        "fraction_pdf_mismatch": {
            "label": "Contribution / <|I|> - PDF",
            "short_label": "delta",
            "format": "scientific",
        },
        "error_contribution": {
            "label": "Error Contribution",
            "short_label": "error contribution",
            "format": "scientific",
        }
    })
}

fn discrete_projection_histogram_views(include_pdf_views: bool) -> Vec<JsonValue> {
    let mut views = vec![json!({
        "id": "value",
        "label": "Value",
        "kind": "bar",
        "value_metric": "value",
        "error_metric": "error",
        "tooltip_metrics": ["value", "error"],
        "default": true,
    })];
    if include_pdf_views {
        views.extend([
            json!({
                "id": "pdf_compare",
                "label": "Compare PDF",
                "kind": "bar_with_marker",
                "value_metric": "value",
                "error_metric": "error",
                "marker_metric": "pdf_scaled_value",
                "delta_metric": "value_pdf_mismatch",
                "tooltip_metrics": ["value", "error", "pdf_scaled_value", "value_pdf_mismatch", "pdf"],
            }),
            json!({
                "id": "pdf_mismatch",
                "label": "PDF Mismatch",
                "kind": "bar",
                "value_metric": "value_pdf_mismatch",
                "label_metric": "value_pdf_mismatch",
                "tooltip_metrics": ["value_pdf_mismatch", "pdf_scaled_value", "value", "pdf"],
            }),
            json!({
                "id": "share",
                "label": "Share vs PDF",
                "kind": "bar_with_marker",
                "value_metric": "contribution_fraction",
                "marker_metric": "pdf",
                "delta_metric": "fraction_pdf_mismatch",
                "tooltip_metrics": ["contribution_fraction", "pdf", "fraction_pdf_mismatch"],
            }),
        ]);
    }
    views
}

fn scalar_projected_bins(
    bins: &BTreeMap<String, DiscreteProjectionBinState>,
    item: &NamedDiscreteProjection,
    projection_name: &str,
    normalization: DiscreteProjectionNormalization,
    total_count: i64,
    bin_limit: usize,
    discrete_pdf: Option<&DiscretePdfCache>,
) -> Result<Vec<JsonValue>, String> {
    let mut projected = BTreeMap::<Vec<i64>, DiscreteProjectionBinState>::new();
    for bin in bins.values() {
        if !matches_fixed_dims(&bin.discrete, item)? {
            continue;
        }
        let Some(key) = projection_key(&bin.discrete, item)? else {
            continue;
        };
        projected
            .entry(key.clone())
            .and_modify(|current| current.merge_in_place(bin.clone()))
            .or_insert_with(|| DiscreteProjectionBinState {
                discrete: key,
                state: bin.state.clone(),
            });
        reject_bin_explosion(projected.len(), bin_limit, &item.name)?;
    }
    let total_abs_contribution = projected
        .values()
        .map(|bin| bin.contribution_mean(total_count).abs())
        .sum::<f64>();
    Ok(projected
        .values()
        .enumerate()
        .map(|(index, bin)| {
            let pdf_cache_available = discrete_pdf.is_some();
            let pdf = discrete_pdf.and_then(|cache| cache.get(projection_name, &bin.discrete));
            let pdf_status = if pdf.is_some() {
                "available"
            } else if pdf_cache_available {
                "missing_bin"
            } else {
                "unavailable"
            };
            let value = scalar_bin_value(bin, normalization, total_count);
            let error = scalar_bin_error(bin, normalization, total_count);
            let relative_error = if value != 0.0 {
                Some((error / value).abs())
            } else {
                None
            };
            let error_contribution = error * error;
            let contribution = bin.contribution_mean(total_count);
            let contribution_error = bin.contribution_stderr(total_count);
            let pdf_scaled_value = pdf.map(|pdf| pdf * total_abs_contribution);
            let value_pdf_mismatch = pdf_scaled_value.map(|scaled| value - scaled);
            let value_pdf_mismatch_abs = value_pdf_mismatch.map(f64::abs);
            let contribution_fraction = if total_abs_contribution > 0.0 {
                Some(contribution / total_abs_contribution)
            } else {
                None
            };
            let fraction_pdf_mismatch =
                contribution_fraction.and_then(|fraction| pdf.map(|pdf| fraction - pdf));
            json!({
                "start": index as f64,
                "stop": index as f64 + 1.0,
                "value": value,
                "error": error,
                "pdf": pdf,
                "pdf_status": pdf_status,
                "pdf_scaled_value": pdf_scaled_value,
                "value_pdf_mismatch": value_pdf_mismatch,
                "relative_error": relative_error,
                "error_contribution": error_contribution,
                "metrics": {
                    "value": {
                        "value": value,
                        "error": error,
                    },
                    "contribution": {
                        "value": contribution,
                        "error": contribution_error,
                    },
                    "conditional_mean": {
                        "value": bin.mean(),
                        "error": bin.stderr(),
                    },
                    "pdf": {
                        "value": pdf,
                    },
                    "pdf_scaled_value": {
                        "value": pdf_scaled_value,
                    },
                    "value_pdf_mismatch": {
                        "value": value_pdf_mismatch,
                    },
                    "value_pdf_mismatch_abs": {
                        "value": value_pdf_mismatch_abs,
                    },
                    "contribution_fraction": {
                        "value": contribution_fraction,
                    },
                    "fraction_pdf_mismatch": {
                        "value": fraction_pdf_mismatch,
                    },
                    "error_contribution": {
                        "value": error_contribution,
                    },
                },
                "label": projection_key_label(&bin.discrete),
                "bin_id": index as i64,
            })
        })
        .collect())
}

fn scalar_bin_value(
    bin: &DiscreteProjectionBinState,
    normalization: DiscreteProjectionNormalization,
    total_count: i64,
) -> f64 {
    match normalization {
        DiscreteProjectionNormalization::Contribution => bin.contribution_mean(total_count),
        DiscreteProjectionNormalization::ConditionalMean => bin.mean(),
    }
}

fn scalar_bin_error(
    bin: &DiscreteProjectionBinState,
    normalization: DiscreteProjectionNormalization,
    total_count: i64,
) -> f64 {
    match normalization {
        DiscreteProjectionNormalization::Contribution => bin.contribution_stderr(total_count),
        DiscreteProjectionNormalization::ConditionalMean => bin.stderr(),
    }
}

fn matches_fixed_dims(discrete: &[i64], item: &NamedDiscreteProjection) -> Result<bool, String> {
    for (raw_dim, fixed_value) in &item.fixed_dims {
        let dim = raw_dim.parse::<usize>().map_err(|_| {
            format!(
                "discrete projection '{}' fixed dimension '{}' is not a non-negative integer dimension index",
                item.name, raw_dim
            )
        })?;
        let Some(actual) = discrete.get(dim) else {
            return Ok(false);
        };
        if actual != fixed_value {
            return Ok(false);
        }
    }
    Ok(true)
}

fn projection_key(
    discrete: &[i64],
    item: &NamedDiscreteProjection,
) -> Result<Option<Vec<i64>>, String> {
    let mut key = Vec::with_capacity(item.dims.len());
    for dim in &item.dims {
        let Some(value) = discrete.get(*dim) else {
            return Ok(None);
        };
        key.push(*value);
    }
    Ok(Some(key))
}

fn reject_bin_explosion(count: usize, bin_limit: usize, name: &str) -> Result<(), String> {
    if count > bin_limit {
        Err(format!(
            "discrete projection '{name}' exceeds fixed bin limit {bin_limit}"
        ))
    } else {
        Ok(())
    }
}

fn discrete_projection_description(item: &NamedDiscreteProjection) -> String {
    let fixed = item
        .fixed_dims
        .iter()
        .map(|(dim, value)| format!("d{dim}={value}"))
        .collect::<Vec<_>>()
        .join(", ");
    if fixed.is_empty() {
        format!("dims={:?}", item.dims)
    } else {
        format!("dims={:?}; fixed {}", item.dims, fixed)
    }
}

fn projection_key_label(key: &[i64]) -> String {
    match key {
        [] => "all".to_string(),
        [value] => value.to_string(),
        values => format!("{values:?}"),
    }
}

struct DiscretePdfCache<'a> {
    projections: &'a serde_json::Map<String, JsonValue>,
}

impl DiscretePdfCache<'_> {
    fn get(&self, projection_name: &str, key: &[i64]) -> Option<f64> {
        self.projections
            .get(projection_name)
            .and_then(JsonValue::as_object)
            .and_then(|projection| projection.get(&discrete_pdf_key(key)))
            .and_then(JsonValue::as_f64)
            .filter(|value| value.is_finite())
    }
}

fn discrete_pdf_cache(diagnostics: &JsonValue) -> Option<DiscretePdfCache<'_>> {
    let cache = diagnostics.get("discrete_pdf")?;
    if cache.get("schema").and_then(JsonValue::as_str) != Some("gammaboard-discrete-pdf-v1") {
        return None;
    }
    Some(DiscretePdfCache {
        projections: cache.get("projections")?.as_object()?,
    })
}

fn discrete_pdf_key(key: &[i64]) -> String {
    serde_json::to_string(key).unwrap_or_else(|_| "[]".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::ScalarAccumulatorState;

    fn discrete_bin(
        discrete: Vec<i64>,
        count: i64,
        sum_weighted_value: f64,
        sum_sq: f64,
    ) -> DiscreteProjectionBinState {
        DiscreteProjectionBinState {
            discrete,
            state: ScalarAccumulatorState {
                count,
                sum_weighted_value,
                sum_sq,
                ..ScalarAccumulatorState::plain()
            },
        }
    }

    fn scalar_projection_fixture() -> (
        BTreeMap<String, DiscreteProjectionBinState>,
        NamedDiscreteProjection,
    ) {
        let mut bins = BTreeMap::new();
        bins.insert("0".to_string(), discrete_bin(vec![0], 2, 6.0, 20.0));
        bins.insert("1".to_string(), discrete_bin(vec![1], 1, -3.0, 9.0));

        let item = NamedDiscreteProjection {
            name: "channel".to_string(),
            dims: vec![0],
            fixed_dims: BTreeMap::new(),
        };

        (bins, item)
    }

    fn projected_values(normalization: DiscreteProjectionNormalization) -> Vec<f64> {
        let (bins, item) = scalar_projection_fixture();
        let projected = scalar_projected_bins(&bins, &item, &item.name, normalization, 3, 16, None)
            .expect("project bins");
        assert_eq!(projected.len(), 2);
        projected
            .iter()
            .map(|bin| bin["value"].as_f64().expect("numeric bin value"))
            .collect::<Vec<_>>()
    }

    #[test]
    fn estimate_history_starts_at_first_non_empty_accumulator() {
        assert!(
            real_estimate_history_panel(AccumulatorState::Scalar(ScalarAccumulatorState::plain()))
                .is_none()
        );

        let panel = real_estimate_history_panel(AccumulatorState::Scalar(ScalarAccumulatorState {
            count: 16,
            sum_weighted_value: 8.0,
            sum_sq: 4.0,
            ..ScalarAccumulatorState::plain()
        }))
        .expect("non-empty accumulator should produce history point");

        let PanelState::ScalarTimeseries { points, .. } = panel else {
            panic!("expected scalar timeseries");
        };
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].x, 16.0);
        assert_eq!(points[0].y, 0.5);
    }

    #[test]
    fn scalar_discrete_projection_contribution_bins_sum_to_integral() {
        let values = projected_values(DiscreteProjectionNormalization::Contribution);
        assert!((values[0] - 2.0).abs() < f64::EPSILON);
        assert!((values[1] + 1.0).abs() < f64::EPSILON);
        assert!((values.iter().sum::<f64>() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn scalar_discrete_projection_conditional_mean_bins_use_bin_counts() {
        let values = projected_values(DiscreteProjectionNormalization::ConditionalMean);
        assert!((values[0] - 3.0).abs() < f64::EPSILON);
        assert!((values[1] + 3.0).abs() < f64::EPSILON);
        assert!((values.iter().sum::<f64>()).abs() < f64::EPSILON);
    }

    #[test]
    fn discrete_projection_skips_samples_outside_variable_depth_path() {
        let mut bins = BTreeMap::new();
        bins.insert("0".to_string(), discrete_bin(vec![0], 10, 10.0, 10.0));
        bins.insert("1/0".to_string(), discrete_bin(vec![1, 0], 10, 20.0, 40.0));
        bins.insert(
            "1/1/3".to_string(),
            discrete_bin(vec![1, 1, 3], 2, 6.0, 20.0),
        );
        let item = NamedDiscreteProjection {
            name: "leaf".to_string(),
            dims: vec![2],
            fixed_dims: BTreeMap::from([("0".to_string(), 1), ("1".to_string(), 1)]),
        };

        let projected = scalar_projected_bins(
            &bins,
            &item,
            &item.name,
            DiscreteProjectionNormalization::ConditionalMean,
            22,
            16,
            None,
        )
        .expect("project variable-depth bins");

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0]["label"], "3");
        assert_eq!(projected[0]["value"], 3.0);
    }
}
