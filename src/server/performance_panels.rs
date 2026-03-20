use crate::core::SamplerRuntimeMetrics;
use crate::server::panels::{
    PanelHistoryMode, PanelKind, PanelResponse, PanelSpec, PanelState, PanelWidth, PlotPoint,
    history_x, merge_panel_state, panel_spec, replace_panel, scalar_timeseries_panel,
    with_panel_width,
};
use crate::stores::{EvaluatorPerformanceHistoryEntry, SamplerPerformanceHistoryEntry};
use std::collections::BTreeMap;

pub fn build_evaluator_performance_response(
    scope_id: Option<String>,
    entries: Vec<EvaluatorPerformanceHistoryEntry>,
) -> PanelResponse {
    build_performance_response(
        scope_id.unwrap_or_else(|| "evaluator".to_string()),
        entries,
        evaluator_panel_specs(),
        |entry| entry.id.to_string(),
        evaluator_panels,
    )
}

pub fn build_sampler_performance_response(
    scope_id: Option<String>,
    entries: Vec<SamplerPerformanceHistoryEntry>,
) -> PanelResponse {
    build_performance_response(
        scope_id.unwrap_or_else(|| "sampler".to_string()),
        entries,
        sampler_panel_specs(),
        |entry| entry.id.to_string(),
        sampler_panels,
    )
}

fn build_performance_response<T>(
    source_id: String,
    entries: Vec<T>,
    panels: Vec<PanelSpec>,
    cursor_for: impl Fn(&T) -> String,
    build_panels: impl Fn(&T) -> Vec<PanelState>,
) -> PanelResponse {
    let cursor = entries.first().map(cursor_for);
    let mut state_by_id = BTreeMap::new();
    for entry in entries.iter().rev() {
        for panel in build_panels(entry) {
            let panel_id = panel.panel_id().to_string();
            if let Some(existing) = state_by_id.get_mut(&panel_id) {
                merge_panel_state(existing, panel);
            } else {
                state_by_id.insert(panel_id, panel);
            }
        }
    }

    PanelResponse {
        source_id,
        cursor,
        reset_required: false,
        panels,
        updates: state_by_id.into_values().map(replace_panel).collect(),
    }
}

fn evaluator_panel_specs() -> Vec<PanelSpec> {
    vec![
        with_panel_width(
            panel_spec(
                "evaluator_idle_ratio",
                "Idle Ratio",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "evaluator_evaluate_time_us",
                "Evaluate Time Per Sample (us)",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "evaluator_parametrization_time_us",
                "Parametrization Time Per Sample (us)",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
    ]
}

fn sampler_panel_specs() -> Vec<PanelSpec> {
    vec![
        with_panel_width(
            panel_spec(
                "sampler_completed_samples_per_second",
                "Completed Samples Per Second",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
        with_panel_width(
            panel_spec(
                "sampler_queue_remaining_ratio",
                "Queue Remaining Ratio",
                PanelKind::ScalarTimeseries,
                PanelHistoryMode::Append,
            ),
            PanelWidth::Full,
        ),
    ]
}

fn evaluator_panels(entry: &EvaluatorPerformanceHistoryEntry) -> Vec<PanelState> {
    let mut panels = vec![
        scalar_point_panel(
            "evaluator_evaluate_time_us",
            history_x(entry.created_at),
            ms_to_us(entry.metrics.avg_evaluate_time_per_sample_ms),
            Some(ms_to_us(
                entry.metrics.avg_evaluate_time_per_sample_ms
                    - entry.metrics.std_evaluate_time_per_sample_ms,
            )),
            Some(ms_to_us(
                entry.metrics.avg_evaluate_time_per_sample_ms
                    + entry.metrics.std_evaluate_time_per_sample_ms,
            )),
        ),
        scalar_point_panel(
            "evaluator_parametrization_time_us",
            history_x(entry.created_at),
            ms_to_us(entry.metrics.avg_parametrization_time_per_sample_ms),
            Some(ms_to_us(
                entry.metrics.avg_parametrization_time_per_sample_ms
                    - entry.metrics.std_parametrization_time_per_sample_ms,
            )),
            Some(ms_to_us(
                entry.metrics.avg_parametrization_time_per_sample_ms
                    + entry.metrics.std_parametrization_time_per_sample_ms,
            )),
        ),
    ];
    if let Some(idle_profile) = entry.metrics.idle_profile.as_ref() {
        panels.insert(
            0,
            scalar_point_panel(
                "evaluator_idle_ratio",
                history_x(entry.created_at),
                idle_profile.idle_ratio,
                None,
                None,
            ),
        );
    }
    panels
}

fn sampler_panels(entry: &SamplerPerformanceHistoryEntry) -> Vec<PanelState> {
    let Some(runtime) = decode_sampler_runtime_metrics(entry) else {
        return Vec::new();
    };
    vec![
        scalar_point_panel(
            "sampler_completed_samples_per_second",
            history_x(entry.created_at),
            runtime.completed_samples_per_second,
            None,
            None,
        ),
        scalar_point_panel(
            "sampler_queue_remaining_ratio",
            history_x(entry.created_at),
            runtime.rolling.queue_remaining_ratio.mean.unwrap_or(0.0),
            Some(
                runtime.rolling.queue_remaining_ratio.mean.unwrap_or(0.0)
                    - runtime.rolling.queue_remaining_ratio.std_dev,
            ),
            Some(
                runtime.rolling.queue_remaining_ratio.mean.unwrap_or(0.0)
                    + runtime.rolling.queue_remaining_ratio.std_dev,
            ),
        ),
    ]
}

fn scalar_point_panel(
    panel_id: &str,
    x: f64,
    y: f64,
    y_min: Option<f64>,
    y_max: Option<f64>,
) -> PanelState {
    scalar_timeseries_panel(panel_id, vec![PlotPoint { x, y, y_min, y_max }])
}

fn decode_sampler_runtime_metrics(
    entry: &SamplerPerformanceHistoryEntry,
) -> Option<SamplerRuntimeMetrics> {
    serde_json::from_value(entry.runtime_metrics.clone()).ok()
}

fn ms_to_us(value_ms: f64) -> f64 {
    value_ms * 1000.0
}
